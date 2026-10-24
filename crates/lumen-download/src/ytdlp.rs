//! yt-dlp runner: spawns the binary, parses progress, supports cancellation.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use lumen_core::CancelFlag;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// One video to fetch. `output_template` ends with `.%(ext)s`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoJob {
    /// May contain a token: never log it unredacted.
    pub url: String,
    pub referer: Option<String>,
    /// Sent as `--user-agent` when set (some CDNs check it).
    pub user_agent: Option<String>,
    pub output_template: PathBuf,
}

/// One generic yt-dlp download whose format and extras are chosen by the
/// platform adapter (`args`), e.g. YouTube quality presets. The template ends
/// with `.%(ext)s`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaJob {
    pub url: String,
    pub output_template: PathBuf,
    /// Extra yt-dlp options placed before `-o` (format, ffmpeg location...).
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VideoError {
    #[error("yt-dlp not found at {0}")]
    NotFound(String),
    #[error("cannot start yt-dlp: {0}")]
    Spawn(String),
    /// yt-dlp exited with an error; carries its last error line.
    #[error("{0}")]
    Failed(String),
    #[error("cancelled")]
    Cancelled,
}

#[async_trait]
pub trait VideoRunner: Send + Sync {
    /// Downloads `job`, reporting 0-100 percentages. Must stop promptly when
    /// `cancel` is set, returning [`VideoError::Cancelled`].
    async fn download(
        &self,
        job: &VideoJob,
        cancel: &CancelFlag,
        on_percent: &(dyn Fn(f32) + Send + Sync),
    ) -> Result<(), VideoError>;
}

/// Metadata resolution and generic downloads, for platforms that are not
/// course sites (YouTube). Behind a trait so adapters are tested with fakes.
#[async_trait]
pub trait MediaTool: Send + Sync {
    /// Runs `yt-dlp -J` and returns its single JSON document. With `flat`,
    /// playlists are listed without resolving each entry (`--flat-playlist`).
    async fn probe(
        &self,
        url: &str,
        flat: bool,
        cancel: &CancelFlag,
    ) -> Result<serde_json::Value, VideoError>;

    /// Downloads `job`, reporting 0-100 percentages, stopping on `cancel`.
    async fn fetch(
        &self,
        job: &MediaJob,
        cancel: &CancelFlag,
        on_percent: &(dyn Fn(f32) + Send + Sync),
    ) -> Result<(), VideoError>;
}

/// Percentage from a `[download]  45.3% of ...` line.
pub fn parse_percent(line: &str) -> Option<f32> {
    let rest = line.trim_start().strip_prefix("[download]")?.trim_start();
    let end = rest.find('%')?;
    rest[..end].trim().parse::<f32>().ok()
}

pub fn build_args(job: &VideoJob) -> Vec<String> {
    let mut args: Vec<String> = [
        "--progress",
        "--newline",
        "-f",
        "best[height<=720]/bestvideo[height<=720]+bestaudio/best",
        "-N",
        "64",
        "--retries",
        "10",
        "--fragment-retries",
        "10",
        "--file-access-retries",
        "30",
        "--buffer-size",
        "64K",
        "--http-chunk-size",
        "10M",
        "--throttled-rate",
        "100K",
        "--merge-output-format",
        "mp4",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();
    if let Some(referer) = &job.referer {
        args.push("--referer".into());
        args.push(referer.clone());
    }
    if let Some(user_agent) = &job.user_agent {
        args.push("--user-agent".into());
        args.push(user_agent.clone());
    }
    args.push("-o".into());
    args.push(job.output_template.to_string_lossy().into_owned());
    args.push("--".into());
    args.push(job.url.clone());
    args
}

pub fn build_media_args(job: &MediaJob) -> Vec<String> {
    let mut args: Vec<String> = [
        "--progress",
        "--newline",
        "--no-playlist",
        "--retries",
        "10",
        "--fragment-retries",
        "10",
        "--file-access-retries",
        "30",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();
    args.extend(job.args.iter().cloned());
    args.push("-o".into());
    args.push(job.output_template.to_string_lossy().into_owned());
    args.push("--".into());
    args.push(job.url.clone());
    args
}

pub fn build_probe_args(url: &str, flat: bool) -> Vec<String> {
    let mut args = vec!["-J".to_string(), "--no-warnings".to_string()];
    if flat {
        // Unavailable entries stay in the listing instead of aborting it.
        args.push("--ignore-errors".into());
        args.push("--flat-playlist".into());
    } else {
        args.push("--no-playlist".into());
    }
    args.push("--".into());
    args.push(url.to_string());
    args
}

/// Splits a byte stream on `\n` and `\r` (yt-dlp redraws progress with `\r`).
#[derive(Debug, Default)]
pub struct LineSplitter {
    buf: Vec<u8>,
}

impl LineSplitter {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        for &b in chunk {
            if b == b'\n' || b == b'\r' {
                if !self.buf.is_empty() {
                    lines.push(String::from_utf8_lossy(&self.buf).into_owned());
                    self.buf.clear();
                }
            } else {
                self.buf.push(b);
            }
        }
        lines
    }

    pub fn finish(self) -> Option<String> {
        (!self.buf.is_empty()).then(|| String::from_utf8_lossy(&self.buf).into_owned())
    }
}

/// Remembers the last line and the last `ERROR:` line of the output.
#[derive(Debug, Default)]
pub struct Tail {
    last_line: Option<String>,
    last_error: Option<String>,
}

impl Tail {
    pub fn feed(&mut self, line: &str) {
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        self.last_line = Some(line.to_string());
        if let Some(err) = line.strip_prefix("ERROR:") {
            self.last_error = Some(err.trim().to_string());
        }
    }

    pub fn message(&self) -> Option<String> {
        self.last_error.clone().or_else(|| self.last_line.clone())
    }
}

/// ffmpeg next to the app (sidecar) or on `path_env`, if any. yt-dlp needs it
/// to merge separate video and audio streams.
pub fn locate_ffmpeg(sidecar_dir: Option<&Path>, path_env: Option<&OsStr>) -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let sidecar = sidecar_dir.map(|d| d.join(name));
    let on_path = path_env
        .into_iter()
        .flat_map(std::env::split_paths)
        .map(|d| d.join(name));
    sidecar.into_iter().chain(on_path).find(|p| p.is_file())
}

/// Removes yt-dlp leftovers (`.part`, `.ytdl`, `.fNNN.`, `.temp.`) for the
/// output template. Finished files are kept.
pub fn clean_partials(output_template: &Path) {
    let (Some(dir), Some(name)) = (output_template.parent(), output_template.file_name()) else {
        return;
    };
    let name = name.to_string_lossy();
    let stem = name.strip_suffix(".%(ext)s").unwrap_or(&name);
    let prefix = format!("{stem}.");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        let Some(rest) = file.strip_prefix(&prefix) else {
            continue;
        };
        let format_part = rest.strip_prefix('f').and_then(|r| r.chars().next());
        let leftover = rest.contains(".part")
            || rest.ends_with(".ytdl")
            || rest.starts_with("temp.")
            || format_part.is_some_and(|c| c.is_ascii_digit());
        if leftover {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Spawns the command, retrying the transient "text file busy" error that
/// appears when a freshly written executable is started.
async fn spawn(cmd: &mut Command, binary: &Path) -> Result<Child, VideoError> {
    const ETXTBSY: i32 = 26;
    let mut attempts = 0;
    loop {
        match cmd.spawn() {
            Ok(child) => return Ok(child),
            Err(e) if e.raw_os_error() == Some(ETXTBSY) && attempts < 10 => {
                attempts += 1;
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(VideoError::NotFound(binary.display().to_string()))
            }
            Err(e) => return Err(VideoError::Spawn(e.to_string())),
        }
    }
}

fn spawn_reader<R>(mut reader: R, tx: mpsc::UnboundedSender<String>) -> JoinHandle<()>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut splitter = LineSplitter::default();
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    for line in splitter.push(&buf[..n]) {
                        if tx.send(line).is_err() {
                            return;
                        }
                    }
                }
            }
        }
        if let Some(line) = splitter.finish() {
            let _ = tx.send(line);
        }
    })
}

#[derive(Debug, Clone)]
pub struct YtDlp {
    binary: PathBuf,
}

impl YtDlp {
    pub fn new(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
        }
    }

    /// The sidecar next to the app when present, otherwise `yt-dlp` from PATH.
    pub fn locate(sidecar_dir: Option<&Path>) -> Self {
        let name = if cfg!(windows) {
            "yt-dlp.exe"
        } else {
            "yt-dlp"
        };
        let sidecar = sidecar_dir.map(|d| d.join(name)).filter(|p| p.is_file());
        Self::new(sidecar.unwrap_or_else(|| PathBuf::from("yt-dlp")))
    }

    pub fn binary(&self) -> &Path {
        &self.binary
    }
}

impl YtDlp {
    fn command(&self, args: Vec<String>) -> Command {
        let mut cmd = Command::new(&self.binary);
        cmd.args(args)
            .env("PYTHONUNBUFFERED", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd
    }

    /// Runs yt-dlp with `args`, streaming percentages; cleans leftovers of
    /// `output_template` when it fails or is cancelled.
    async fn run(
        &self,
        args: Vec<String>,
        output_template: &Path,
        cancel: &CancelFlag,
        on_percent: &(dyn Fn(f32) + Send + Sync),
    ) -> Result<(), VideoError> {
        if cancel.is_cancelled() {
            return Err(VideoError::Cancelled);
        }
        let mut cmd = self.command(args);
        let mut child = spawn(&mut cmd, &self.binary).await?;

        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let mut readers = Vec::new();
        if let Some(out) = child.stdout.take() {
            readers.push(spawn_reader(out, tx.clone()));
        }
        if let Some(err) = child.stderr.take() {
            readers.push(spawn_reader(err, tx.clone()));
        }
        drop(tx);

        let mut tail = Tail::default();
        let mut tick = tokio::time::interval(Duration::from_millis(50));
        loop {
            tokio::select! {
                line = rx.recv() => match line {
                    Some(line) => {
                        if let Some(p) = parse_percent(&line) {
                            on_percent(p);
                        }
                        tail.feed(&line);
                    }
                    None => break,
                },
                _ = tick.tick() => {
                    if cancel.is_cancelled() {
                        let _ = child.kill().await;
                        readers.iter().for_each(JoinHandle::abort);
                        clean_partials(output_template);
                        return Err(VideoError::Cancelled);
                    }
                }
            }
        }
        let status = child
            .wait()
            .await
            .map_err(|e| VideoError::Spawn(e.to_string()))?;
        if status.success() {
            return Ok(());
        }
        clean_partials(output_template);
        Err(VideoError::Failed(
            tail.message()
                .unwrap_or_else(|| format!("yt-dlp exited with {status}")),
        ))
    }
}

#[async_trait]
impl VideoRunner for YtDlp {
    async fn download(
        &self,
        job: &VideoJob,
        cancel: &CancelFlag,
        on_percent: &(dyn Fn(f32) + Send + Sync),
    ) -> Result<(), VideoError> {
        self.run(build_args(job), &job.output_template, cancel, on_percent)
            .await
    }
}

#[async_trait]
impl MediaTool for YtDlp {
    async fn probe(
        &self,
        url: &str,
        flat: bool,
        cancel: &CancelFlag,
    ) -> Result<serde_json::Value, VideoError> {
        if cancel.is_cancelled() {
            return Err(VideoError::Cancelled);
        }
        let mut cmd = self.command(build_probe_args(url, flat));
        let child = spawn(&mut cmd, &self.binary).await?;
        // Dropping the future kills the child (`kill_on_drop`).
        let mut output = Box::pin(child.wait_with_output());
        let mut tick = tokio::time::interval(Duration::from_millis(50));
        let output = loop {
            tokio::select! {
                out = &mut output => break out.map_err(|e| VideoError::Spawn(e.to_string()))?,
                _ = tick.tick() => {
                    if cancel.is_cancelled() {
                        return Err(VideoError::Cancelled);
                    }
                }
            }
        };
        if !output.status.success() {
            let mut tail = Tail::default();
            String::from_utf8_lossy(&output.stderr)
                .lines()
                .for_each(|l| tail.feed(l));
            return Err(VideoError::Failed(tail.message().unwrap_or_else(|| {
                format!("yt-dlp exited with {}", output.status)
            })));
        }
        serde_json::from_slice(&output.stdout)
            .map_err(|e| VideoError::Failed(format!("yt-dlp returned invalid JSON: {e}")))
    }

    async fn fetch(
        &self,
        job: &MediaJob,
        cancel: &CancelFlag,
        on_percent: &(dyn Fn(f32) + Send + Sync),
    ) -> Result<(), VideoError> {
        self.run(
            build_media_args(job),
            &job.output_template,
            cancel,
            on_percent,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn job(dir: &Path, referer: Option<&str>) -> VideoJob {
        VideoJob {
            url: "https://stream.mux.com/p.m3u8?token=T".into(),
            referer: referer.map(str::to_string),
            user_agent: None,
            output_template: dir.join("01 - Intro.%(ext)s"),
        }
    }

    #[test]
    fn percent_is_parsed_from_download_lines_only() {
        assert_eq!(
            parse_percent("[download]  45.3% of 10.00MiB at 1MiB/s"),
            Some(45.3)
        );
        assert_eq!(parse_percent("[download] 100% of 10MiB"), Some(100.0));
        assert_eq!(parse_percent("[download] Destination: x.mp4"), None);
        assert_eq!(parse_percent("[info] 50% something"), None);
    }

    #[test]
    fn both_recipes_retry_file_access_for_windows_scanners() {
        // Windows antivirus/indexers hold the .part file briefly; yt-dlp's
        // default of 3 retries gave up with WinError 32 on the final rename.
        let video = build_args(&job(Path::new("/x"), None));
        let media = build_media_args(&media_job(Path::new("/x")));
        for args in [video, media] {
            let at = args.iter().position(|a| a == "--file-access-retries");
            let at = at.expect("--file-access-retries is set");
            assert_eq!(args[at + 1], "30");
        }
    }

    #[test]
    fn args_follow_the_proven_recipe() {
        let tmp = Path::new("/x");
        let a = build_args(&job(tmp, Some("https://www.skool.com/")));
        let pos = |s: &str| a.iter().position(|x| x == s);
        assert_eq!(a[pos("--referer").unwrap() + 1], "https://www.skool.com/");
        assert!(pos("--newline").is_some());
        assert!(pos("--no-check-certificates").is_none());
        assert_eq!(a[pos("--merge-output-format").unwrap() + 1], "mp4");
        let n = a.len();
        assert_eq!(a[n - 2], "--");
        assert_eq!(a[n - 1], "https://stream.mux.com/p.m3u8?token=T");
        assert_eq!(
            a[pos("-o").unwrap() + 1],
            Path::new("/x").join("01 - Intro.%(ext)s").to_string_lossy()
        );
        assert!(build_args(&job(tmp, None)).iter().all(|x| x != "--referer"));
    }

    #[test]
    fn a_user_agent_is_passed_only_when_set() {
        let tmp = Path::new("/x");
        let mut j = job(tmp, None);
        assert!(build_args(&j).iter().all(|x| x != "--user-agent"));
        j.user_agent = Some("UA/1.0".into());
        let a = build_args(&j);
        let at = a.iter().position(|x| x == "--user-agent").unwrap();
        assert_eq!(a[at + 1], "UA/1.0");
        assert!(at < a.len() - 2, "options come before the `--` separator");
    }

    #[test]
    fn splitter_handles_cr_lf_crlf_and_partial_chunks() {
        let mut s = LineSplitter::default();
        assert_eq!(s.push(b"a\rb\n\r\nc"), ["a", "b"]);
        assert_eq!(s.push(b"d\n"), ["cd"]);
        assert_eq!(s.push(b"tail"), Vec::<String>::new());
        assert_eq!(s.finish().as_deref(), Some("tail"));
        assert_eq!(LineSplitter::default().finish(), None);
    }

    #[test]
    fn tail_prefers_the_last_error_line() {
        let mut t = Tail::default();
        assert_eq!(t.message(), None);
        t.feed("[download] 10%");
        assert_eq!(t.message().as_deref(), Some("[download] 10%"));
        t.feed("ERROR: Unable to download: HTTP 403");
        t.feed("some trailing note");
        assert_eq!(t.message().as_deref(), Some("Unable to download: HTTP 403"));
    }

    #[test]
    fn partials_are_cleaned_but_finished_files_stay() {
        let tmp = tempfile::tempdir().unwrap();
        for f in [
            "01 - Intro.mp4.part",
            "01 - Intro.f248.mp4",
            "01 - Intro.temp.mp4",
            "01 - Intro.mp4.ytdl",
            "01 - Intro.mp4",
            "02 - Other.f1.mp4",
            "description.html",
        ] {
            std::fs::write(tmp.path().join(f), "x").unwrap();
        }
        clean_partials(&tmp.path().join("01 - Intro.%(ext)s"));
        let mut left: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            ["01 - Intro.mp4", "02 - Other.f1.mp4", "description.html"]
        );
    }

    #[test]
    fn fragment_leftovers_are_cleaned_too() {
        let tmp = tempfile::tempdir().unwrap();
        for f in [
            "1. Lesson.mp4.part-Frag190",
            "1. Lesson.mp4.part-Frag195.part",
            "1. Lesson.mp4",
            "10. Lesson.mp4.part-Frag1",
        ] {
            std::fs::write(tmp.path().join(f), "x").unwrap();
        }
        clean_partials(&tmp.path().join("1. Lesson.%(ext)s"));
        let mut left: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["1. Lesson.mp4", "10. Lesson.mp4.part-Frag1"]);
    }

    #[test]
    fn ffmpeg_is_found_in_the_sidecar_dir_first_then_on_the_path() {
        let name = if cfg!(windows) {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        };
        let side = tempfile::tempdir().unwrap();
        let bin = tempfile::tempdir().unwrap();
        let path = std::env::join_paths([bin.path()]).unwrap();
        assert_eq!(locate_ffmpeg(Some(side.path()), Some(&path)), None);
        assert_eq!(locate_ffmpeg(None, None), None);
        std::fs::write(bin.path().join(name), "x").unwrap();
        assert_eq!(
            locate_ffmpeg(Some(side.path()), Some(&path)),
            Some(bin.path().join(name))
        );
        std::fs::write(side.path().join(name), "x").unwrap();
        assert_eq!(
            locate_ffmpeg(Some(side.path()), Some(&path)),
            Some(side.path().join(name))
        );
    }

    #[test]
    fn locate_prefers_an_existing_sidecar() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            YtDlp::locate(Some(tmp.path())).binary(),
            Path::new("yt-dlp")
        );
        assert_eq!(YtDlp::locate(None).binary(), Path::new("yt-dlp"));
        let name = if cfg!(windows) {
            "yt-dlp.exe"
        } else {
            "yt-dlp"
        };
        std::fs::write(tmp.path().join(name), "x").unwrap();
        assert_eq!(
            YtDlp::locate(Some(tmp.path())).binary(),
            tmp.path().join(name)
        );
    }

    #[tokio::test]
    async fn missing_binary_is_reported_by_path() {
        let tmp = tempfile::tempdir().unwrap();
        let y = YtDlp::new(tmp.path().join("nope"));
        let err = y
            .download(&job(tmp.path(), None), &CancelFlag::new(), &|_| {})
            .await
            .unwrap_err();
        assert!(matches!(err, VideoError::NotFound(_)), "{err:?}");
    }

    fn media_job(dir: &Path) -> MediaJob {
        MediaJob {
            url: "https://www.youtube.com/watch?v=abcdefghijk".into(),
            output_template: dir.join("Title.%(ext)s"),
            args: vec!["-f".into(), "b[height<=720]".into()],
        }
    }

    #[test]
    fn media_args_put_adapter_args_before_the_output_and_url() {
        let a = build_media_args(&media_job(Path::new("/x")));
        let pos = |s: &str| a.iter().position(|x| x == s).unwrap();
        assert_eq!(a[pos("-f") + 1], "b[height<=720]");
        assert!(a.contains(&"--newline".to_string()));
        assert!(a.contains(&"--no-playlist".to_string()));
        assert_eq!(
            a[pos("-o") + 1],
            Path::new("/x").join("Title.%(ext)s").to_string_lossy()
        );
        let n = a.len();
        assert_eq!(a[n - 2], "--");
        assert_eq!(a[n - 1], "https://www.youtube.com/watch?v=abcdefghijk");
        assert!(pos("-f") < pos("-o"));
    }

    #[test]
    fn probe_args_ask_for_one_json_document() {
        let flat = build_probe_args("https://x.test/p", true);
        assert_eq!(
            flat,
            [
                "-J",
                "--no-warnings",
                "--ignore-errors",
                "--flat-playlist",
                "--",
                "https://x.test/p"
            ]
        );
        let full = build_probe_args("https://x.test/v", false);
        assert_eq!(
            full,
            [
                "-J",
                "--no-warnings",
                "--no-playlist",
                "--",
                "https://x.test/v"
            ]
        );
    }

    #[tokio::test]
    async fn probe_and_fetch_report_a_missing_binary() {
        let tmp = tempfile::tempdir().unwrap();
        let y = YtDlp::new(tmp.path().join("nope"));
        let c = CancelFlag::new();
        assert!(matches!(
            y.probe("https://x.test", true, &c).await.unwrap_err(),
            VideoError::NotFound(_)
        ));
        assert!(matches!(
            y.fetch(&media_job(tmp.path()), &c, &|_| {})
                .await
                .unwrap_err(),
            VideoError::NotFound(_)
        ));
    }

    #[cfg(unix)]
    mod fake_binary {
        use super::*;
        use std::os::unix::fs::PermissionsExt;
        use std::time::{Duration, Instant};

        fn script(dir: &Path, body: &str) -> YtDlp {
            let path = dir.join("fake-yt-dlp");
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            YtDlp::new(path)
        }

        const OUTPUT_FILE: &str = r#"
while [ $# -gt 0 ]; do
  if [ "$1" = "-o" ]; then out="$2"; fi
  shift
done
"#;

        #[tokio::test]
        async fn success_reports_progress_and_creates_the_file() {
            let tmp = tempfile::tempdir().unwrap();
            let y = script(
                tmp.path(),
                &format!(
                    r#"{OUTPUT_FILE}
printf '[download]   5.0%% of 1MiB\r[download]  50.5%% of 1MiB\r[download] 100%% of 1MiB\n'
f=$(printf '%s' "$out" | sed 's/%(ext)s/mp4/')
echo video > "$f""#
                ),
            );
            let seen = Mutex::new(Vec::new());
            y.download(&job(tmp.path(), None), &CancelFlag::new(), &|p| {
                seen.lock().unwrap().push(p);
            })
            .await
            .unwrap();
            assert_eq!(*seen.lock().unwrap(), [5.0, 50.5, 100.0]);
            assert!(tmp.path().join("01 - Intro.mp4").is_file());
        }

        #[tokio::test]
        async fn failure_carries_the_last_error_line() {
            let tmp = tempfile::tempdir().unwrap();
            let y = script(
                tmp.path(),
                "echo 'ERROR: HTTP Error 403: Forbidden' >&2\nexit 1",
            );
            let err = y
                .download(&job(tmp.path(), None), &CancelFlag::new(), &|_| {})
                .await
                .unwrap_err();
            assert_eq!(err, VideoError::Failed("HTTP Error 403: Forbidden".into()));
        }

        #[tokio::test]
        async fn cancel_kills_the_process_promptly() {
            let tmp = tempfile::tempdir().unwrap();
            let y = script(tmp.path(), "exec sleep 30");
            let cancel = CancelFlag::new();
            let c2 = cancel.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(200)).await;
                c2.cancel();
            });
            let t = Instant::now();
            let err = y
                .download(&job(tmp.path(), None), &cancel, &|_| {})
                .await
                .unwrap_err();
            assert_eq!(err, VideoError::Cancelled);
            assert!(t.elapsed() < Duration::from_secs(5));
        }

        #[tokio::test]
        async fn exit_without_output_still_reports_a_failure() {
            let tmp = tempfile::tempdir().unwrap();
            let y = script(tmp.path(), "exit 3");
            let err = y
                .download(&job(tmp.path(), None), &CancelFlag::new(), &|_| {})
                .await
                .unwrap_err();
            assert!(matches!(err, VideoError::Failed(m) if m.contains('3')));
        }

        #[tokio::test]
        async fn probe_returns_the_parsed_json_and_passes_the_flat_flags() {
            let tmp = tempfile::tempdir().unwrap();
            let argv = tmp.path().join("argv.txt");
            let y = script(
                tmp.path(),
                &format!(
                    r#"printf '%s\n' "$@" > '{}'
echo '{{"_type":"playlist","title":"Mi lista ñ","entries":[]}}'"#,
                    argv.display()
                ),
            );
            let v = y
                .probe(
                    "https://www.youtube.com/playlist?list=PL12345678901",
                    true,
                    &CancelFlag::new(),
                )
                .await
                .unwrap();
            assert_eq!(v["title"], "Mi lista ñ");
            let args = std::fs::read_to_string(argv).unwrap();
            assert!(args.contains("--flat-playlist") && args.contains("-J"));
            assert!(args.ends_with("playlist?list=PL12345678901\n"));
        }

        #[tokio::test]
        async fn probe_failure_carries_the_error_line_and_bad_json_is_a_failure() {
            let tmp = tempfile::tempdir().unwrap();
            let y = script(
                tmp.path(),
                "echo 'ERROR: [youtube] abc: Video unavailable. This video is private' >&2\nexit 1",
            );
            let err = y.probe("u", false, &CancelFlag::new()).await.unwrap_err();
            assert_eq!(
                err,
                VideoError::Failed(
                    "[youtube] abc: Video unavailable. This video is private".into()
                )
            );
            let y = script(tmp.path(), "echo not-json");
            let err = y.probe("u", false, &CancelFlag::new()).await.unwrap_err();
            assert!(matches!(err, VideoError::Failed(m) if m.contains("JSON")));
        }

        #[tokio::test]
        async fn probe_stops_promptly_on_cancel() {
            let tmp = tempfile::tempdir().unwrap();
            let y = script(tmp.path(), "exec sleep 30");
            let cancel = CancelFlag::new();
            let c2 = cancel.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(200)).await;
                c2.cancel();
            });
            let t = Instant::now();
            let err = y.probe("u", true, &cancel).await.unwrap_err();
            assert_eq!(err, VideoError::Cancelled);
            assert!(t.elapsed() < Duration::from_secs(5));
        }

        #[tokio::test]
        async fn fetch_reports_progress_runs_adapter_args_and_can_be_cancelled() {
            let tmp = tempfile::tempdir().unwrap();
            let y = script(
                tmp.path(),
                &format!(
                    r#"{OUTPUT_FILE}
printf '[download]  50.0%% of 1MiB\n'
f=$(printf '%s' "$out" | sed 's/%(ext)s/mp4/')
echo video > "$f""#
                ),
            );
            let seen = Mutex::new(Vec::new());
            y.fetch(&media_job(tmp.path()), &CancelFlag::new(), &|p| {
                seen.lock().unwrap().push(p);
            })
            .await
            .unwrap();
            assert_eq!(*seen.lock().unwrap(), [50.0]);
            assert!(tmp.path().join("Title.mp4").is_file());
            let cancelled = CancelFlag::new();
            cancelled.cancel();
            assert_eq!(
                y.fetch(&media_job(tmp.path()), &cancelled, &|_| {})
                    .await
                    .unwrap_err(),
                VideoError::Cancelled
            );
        }
    }
}
