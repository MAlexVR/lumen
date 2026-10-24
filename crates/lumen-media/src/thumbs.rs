//! Cached video thumbnails made by ffmpeg and served as plain images, so the
//! library grid never needs a `<video>` per card. One JPEG per (library path,
//! size, mtime, time, width): a changed file gets a new key and the old one is
//! cleaned up by the size cap. Requests return at once; ffmpeg runs in a
//! bounded pool (newest request first, because the user scrolls) and finished
//! items are announced through a callback.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::process::Command;

use crate::confine::resolve_confined;

pub const DEFAULT_AT_SECS: f64 = 8.0;
pub const DEFAULT_WIDTH: u32 = 480;
const MIN_WIDTH: u32 = 64;
const MAX_WIDTH: u32 = 1920;
const MAX_AT_SECS: f64 = 1_000_000.0;

#[derive(Debug, Clone)]
pub struct ThumbConfig {
    /// Folder holding `<hash>.jpg` files (created on demand).
    pub cache_dir: PathBuf,
    /// ffmpeg binary; `None` makes every request `unavailable`.
    pub ffmpeg: Option<PathBuf>,
    /// Concurrent ffmpeg processes.
    pub workers: usize,
    /// Per ffmpeg run, then it is killed.
    pub timeout: Duration,
    pub cap_bytes: u64,
    /// How long a failure is remembered before retrying.
    pub negative_ttl: Duration,
}

impl ThumbConfig {
    pub fn new(cache_dir: PathBuf, ffmpeg: Option<PathBuf>) -> Self {
        Self {
            cache_dir,
            ffmpeg,
            workers: 2,
            timeout: Duration::from_secs(15),
            cap_bytes: 300 * 1024 * 1024,
            negative_ttl: Duration::from_secs(60),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThumbItem {
    pub path: String,
    pub at_secs: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ThumbState {
    Ready,
    Pending,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThumbReply {
    pub path: String,
    pub at_secs: f64,
    pub state: ThumbState,
    /// `<hash>.jpg` inside the cache folder when ready.
    pub file: Option<String>,
    /// `invalid_path`, `ffmpeg_missing`, `ffmpeg_failed`, `timeout`.
    pub reason: Option<String>,
}

/// A pending item finished: the file name, or the reason it is unavailable.
#[derive(Debug, Clone, PartialEq)]
pub struct ThumbDone {
    pub path: String,
    pub at_secs: f64,
    pub result: Result<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThumbStatus {
    pub ffmpeg_available: bool,
    pub cache_bytes: u64,
    pub queued: usize,
    pub running: usize,
}

type OnDone = Arc<dyn Fn(ThumbDone) + Send + Sync>;

struct Job {
    key: String,
    rel: String,
    full: PathBuf,
    at: f64,
    width: u32,
}

#[derive(Default)]
struct State {
    queue: Vec<Job>,
    running: usize,
    inflight: HashSet<String>,
    negative: HashMap<String, (Instant, String)>,
}

struct Inner {
    cfg: ThumbConfig,
    state: Mutex<State>,
    on_done: OnDone,
}

#[derive(Clone)]
pub struct ThumbService {
    inner: Arc<Inner>,
}

/// Stable cache key: changing the file (size or mtime), the moment or the
/// width gives another thumbnail.
pub fn thumb_key(rel: &str, size: u64, mtime_nanos: u128, at: f64, width: u32) -> String {
    let mut h = Sha256::new();
    h.update(format!("{rel}|{size}|{mtime_nanos}|{at:.3}|{width}"));
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn lock(m: &Mutex<State>) -> std::sync::MutexGuard<'_, State> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Sum of the `<hash>.jpg` sizes in `dir`.
pub(crate) fn cache_bytes(dir: &Path) -> u64 {
    cache_files(dir).iter().map(|(_, size, _)| size).sum()
}

fn is_cache_file(name: &str) -> bool {
    name.ends_with(".jpg") && !name.ends_with(".part.jpg")
}

/// `(path, size, modified)` of every finished thumbnail.
fn cache_files(dir: &Path) -> Vec<(PathBuf, u64, SystemTime)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| is_cache_file(&e.file_name().to_string_lossy()))
        .filter_map(|e| {
            let meta = e.metadata().ok().filter(std::fs::Metadata::is_file)?;
            Some((
                e.path(),
                meta.len(),
                meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            ))
        })
        .collect()
}

/// Removes stray `.part.jpg` files and, oldest first, finished thumbnails
/// until the folder fits in `cap` bytes.
pub(crate) fn enforce_cap(dir: &Path, cap: u64) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().ends_with(".part.jpg") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let mut files = cache_files(dir);
    let mut total: u64 = files.iter().map(|(_, s, _)| s).sum();
    files.sort_by_key(|(_, _, modified)| *modified);
    for (path, size, _) in files {
        if total <= cap {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= size;
        }
    }
}

fn clamp_at(at: Option<f64>) -> f64 {
    match at {
        Some(t) if t.is_finite() => t.clamp(0.0, MAX_AT_SECS),
        _ => DEFAULT_AT_SECS,
    }
}

fn mtime_nanos(meta: &std::fs::Metadata) -> u128 {
    meta.modified()
        .ok()
        .and_then(|m| m.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos())
}

/// Spawns `cmd`, retrying the transient "text file busy" error of a freshly
/// written executable.
async fn spawn(cmd: &mut Command) -> std::io::Result<tokio::process::Child> {
    const ETXTBSY: i32 = 26;
    let mut attempts = 0;
    loop {
        match cmd.spawn() {
            Err(e) if e.raw_os_error() == Some(ETXTBSY) && attempts < 10 => {
                attempts += 1;
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            other => return other,
        }
    }
}

enum Attempt {
    Produced,
    Empty,
    Failed(&'static str),
}

/// One ffmpeg run into `part`. A clean exit without a frame (the file is
/// shorter than `at`) is `Empty`.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

async fn run_ffmpeg(
    ffmpeg: &Path,
    input: &Path,
    at: f64,
    width: u32,
    part: &Path,
    timeout: Duration,
) -> Attempt {
    let _ = std::fs::remove_file(part);
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-nostdin", "-hide_banner", "-loglevel", "error", "-ss"])
        .arg(format!("{at}"))
        .arg("-i")
        .arg(input)
        .args(["-frames:v", "1", "-vf"])
        .arg(format!("scale={width}:-2"))
        .args(["-q:v", "5"])
        .arg(part)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    // Without this Windows flashes a console window for every thumbnail.
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let mut child = match spawn(&mut cmd).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Attempt::Failed("ffmpeg_missing")
        }
        Err(_) => return Attempt::Failed("ffmpeg_failed"),
    };
    let status = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(_)) => return Attempt::Failed("ffmpeg_failed"),
        Err(_) => {
            let _ = child.kill().await;
            let _ = std::fs::remove_file(part);
            return Attempt::Failed("timeout");
        }
    };
    let produced = status.success() && std::fs::metadata(part).is_ok_and(|m| m.len() > 0);
    if produced {
        Attempt::Produced
    } else {
        let _ = std::fs::remove_file(part);
        Attempt::Empty
    }
}

async fn make(inner: &Inner, job: &Job) -> Result<String, String> {
    let Some(ffmpeg) = &inner.cfg.ffmpeg else {
        return Err("ffmpeg_missing".into());
    };
    let dir = &inner.cfg.cache_dir;
    std::fs::create_dir_all(dir).map_err(|_| "ffmpeg_failed".to_string())?;
    let part = dir.join(format!("{}.part.jpg", job.key));
    let mut at = job.at;
    loop {
        match run_ffmpeg(ffmpeg, &job.full, at, job.width, &part, inner.cfg.timeout).await {
            Attempt::Produced => break,
            Attempt::Failed(reason) => return Err(reason.to_string()),
            Attempt::Empty if at > 0.0 => at = 0.0,
            Attempt::Empty => return Err("ffmpeg_failed".into()),
        }
    }
    let file = format!("{}.jpg", job.key);
    std::fs::rename(&part, dir.join(&file)).map_err(|_| {
        let _ = std::fs::remove_file(&part);
        "ffmpeg_failed".to_string()
    })?;
    Ok(file)
}

fn kick(inner: &Arc<Inner>) {
    loop {
        let job = {
            let mut st = lock(&inner.state);
            if st.running >= inner.cfg.workers.max(1) {
                return;
            }
            let Some(job) = st.queue.pop() else {
                return;
            };
            st.running += 1;
            job
        };
        let inner = Arc::clone(inner);
        tokio::spawn(async move {
            let result = make(&inner, &job).await;
            let idle = {
                let mut st = lock(&inner.state);
                st.running -= 1;
                st.inflight.remove(&job.key);
                if let Err(reason) = &result {
                    st.negative
                        .insert(job.key.clone(), (Instant::now(), reason.clone()));
                }
                st.running == 0 && st.queue.is_empty()
            };
            (inner.on_done)(ThumbDone {
                path: job.rel.clone(),
                at_secs: job.at,
                result,
            });
            kick(&inner);
            if idle {
                let dir = inner.cfg.cache_dir.clone();
                let cap = inner.cfg.cap_bytes;
                let _ = tokio::task::spawn_blocking(move || enforce_cap(&dir, cap)).await;
            }
        });
    }
}

impl ThumbService {
    /// Cleans an oversized cache right away. Needs a Tokio runtime for work.
    pub fn new(cfg: ThumbConfig, on_done: OnDone) -> Self {
        enforce_cap(&cfg.cache_dir, cfg.cap_bytes);
        Self {
            inner: Arc::new(Inner {
                cfg,
                state: Mutex::new(State::default()),
                on_done,
            }),
        }
    }

    pub fn cache_dir(&self) -> &Path {
        &self.inner.cfg.cache_dir
    }

    fn reply(
        path: &str,
        at: f64,
        state: ThumbState,
        file: Option<String>,
        reason: Option<&str>,
    ) -> ThumbReply {
        ThumbReply {
            path: path.to_string(),
            at_secs: at,
            state,
            file,
            reason: reason.map(str::to_string),
        }
    }

    /// Answers at once: `ready` from the cache, `pending` when queued or
    /// already running, `unavailable` for bad paths, a missing ffmpeg or a
    /// recent failure. Paths are confined to regular files under `root`.
    pub fn request(&self, root: &Path, items: &[ThumbItem], width: Option<u32>) -> Vec<ThumbReply> {
        let width = width.unwrap_or(DEFAULT_WIDTH).clamp(MIN_WIDTH, MAX_WIDTH);
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            let at = clamp_at(item.at_secs);
            let unavailable = |reason: &str| {
                Self::reply(&item.path, at, ThumbState::Unavailable, None, Some(reason))
            };
            let Some(full) = resolve_confined(root, &item.path) else {
                out.push(unavailable("invalid_path"));
                continue;
            };
            let Ok(meta) = std::fs::metadata(&full) else {
                out.push(unavailable("invalid_path"));
                continue;
            };
            let key = thumb_key(&item.path, meta.len(), mtime_nanos(&meta), at, width);
            let file = format!("{key}.jpg");
            if self.inner.cfg.cache_dir.join(&file).is_file() {
                out.push(Self::reply(
                    &item.path,
                    at,
                    ThumbState::Ready,
                    Some(file),
                    None,
                ));
                continue;
            }
            if self.inner.cfg.ffmpeg.is_none() {
                out.push(unavailable("ffmpeg_missing"));
                continue;
            }
            let mut st = lock(&self.inner.state);
            if let Some((when, reason)) = st.negative.get(&key).cloned() {
                if when.elapsed() < self.inner.cfg.negative_ttl {
                    out.push(unavailable(&reason));
                    continue;
                }
                st.negative.remove(&key);
            }
            if st.inflight.insert(key.clone()) {
                st.queue.push(Job {
                    key,
                    rel: item.path.clone(),
                    full,
                    at,
                    width,
                });
            }
            drop(st);
            out.push(Self::reply(&item.path, at, ThumbState::Pending, None, None));
        }
        kick(&self.inner);
        out
    }

    /// Drops queued (not running) items of these library paths.
    pub fn cancel(&self, paths: &[String]) {
        let mut st = lock(&self.inner.state);
        let dropped: Vec<String> = st
            .queue
            .iter()
            .filter(|j| paths.contains(&j.rel))
            .map(|j| j.key.clone())
            .collect();
        st.queue.retain(|j| !paths.contains(&j.rel));
        for key in dropped {
            st.inflight.remove(&key);
        }
    }

    pub fn status(&self) -> ThumbStatus {
        let st = lock(&self.inner.state);
        ThumbStatus {
            ffmpeg_available: self.inner.cfg.ffmpeg.as_ref().is_some_and(|p| p.is_file()),
            cache_bytes: cache_bytes(&self.inner.cfg.cache_dir),
            queued: st.queue.len(),
            running: st.running,
        }
    }
}

// The fake ffmpeg is a shell script.
#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tokio::sync::mpsc;

    struct Env {
        _tmp: tempfile::TempDir,
        root: PathBuf,
        cache: PathBuf,
        log: PathBuf,
        bin: PathBuf,
    }

    /// A fake ffmpeg: logs `start <input> <ss>` / `end` and writes the output
    /// argument, behaving as `body` says.
    fn env(body: &str) -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("lib");
        std::fs::create_dir_all(root.join("Skool/G")).unwrap();
        for n in ["a", "b", "c", "d", "e", "f"] {
            std::fs::write(root.join(format!("Skool/G/{n}.mp4")), format!("video {n}")).unwrap();
        }
        let log = tmp.path().join("ffmpeg.log");
        let bin = tmp.path().join("fake-ffmpeg");
        let script = format!(
            r#"#!/bin/sh
printf '%s\n' "$@" > '{log}.args'
while [ $# -gt 0 ]; do
  case "$1" in -i) input="$2";; -ss) ss="$2";; esac
  out="$1"; shift
done
printf 'start %s %s\n' "$input" "$ss" >> '{log}'
{body}
printf 'end\n' >> '{log}'
"#,
            log = log.display()
        );
        std::fs::write(&bin, script).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let cache = tmp.path().join("cache/thumbs");
        Env {
            _tmp: tmp,
            root,
            cache,
            log,
            bin,
        }
    }

    const WRITE: &str = r#"printf 'jpg' > "$out""#;
    const FAIL: &str = "exit 1";

    fn cfg(e: &Env) -> ThumbConfig {
        ThumbConfig {
            cache_dir: e.cache.clone(),
            ffmpeg: Some(e.bin.clone()),
            workers: 2,
            timeout: Duration::from_secs(10),
            cap_bytes: 300 * 1024 * 1024,
            negative_ttl: Duration::from_secs(60),
        }
    }

    type Rx = mpsc::UnboundedReceiver<ThumbDone>;

    fn service(c: ThumbConfig) -> (ThumbService, Rx) {
        let (tx, rx) = mpsc::unbounded_channel();
        let svc = ThumbService::new(
            c,
            Arc::new(move |d| {
                let _ = tx.send(d);
            }),
        );
        (svc, rx)
    }

    async fn done(rx: &mut Rx, n: usize) -> Vec<ThumbDone> {
        let mut out = Vec::new();
        while out.len() < n {
            let d = tokio::time::timeout(Duration::from_secs(10), rx.recv())
                .await
                .expect("event in time")
                .expect("channel open");
            out.push(d);
        }
        out
    }

    fn item(p: &str) -> ThumbItem {
        ThumbItem {
            path: p.into(),
            at_secs: None,
        }
    }

    fn log_lines(e: &Env) -> Vec<String> {
        std::fs::read_to_string(&e.log)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn runs(e: &Env) -> usize {
        log_lines(e)
            .iter()
            .filter(|l| l.starts_with("start"))
            .count()
    }

    #[test]
    fn keys_depend_on_path_size_mtime_time_and_width() {
        let base = thumb_key("a/b.mp4", 10, 1000, 8.0, 480);
        assert_eq!(base, thumb_key("a/b.mp4", 10, 1000, 8.0, 480));
        assert_eq!(base.len(), 64);
        assert!(base
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
        for other in [
            thumb_key("a/c.mp4", 10, 1000, 8.0, 480),
            thumb_key("a/b.mp4", 11, 1000, 8.0, 480),
            thumb_key("a/b.mp4", 10, 1001, 8.0, 480),
            thumb_key("a/b.mp4", 10, 1000, 9.0, 480),
            thumb_key("a/b.mp4", 10, 1000, 8.0, 320),
        ] {
            assert_ne!(base, other);
        }
    }

    #[tokio::test]
    async fn a_request_becomes_ready_once_and_later_hits_the_cache_without_ffmpeg() {
        let e = env(WRITE);
        let (svc, mut rx) = service(cfg(&e));
        let first = svc.request(&e.root, &[item("Skool/G/a.mp4")], None);
        assert_eq!(first[0].state, ThumbState::Pending);
        assert_eq!(
            (first[0].path.as_str(), first[0].at_secs),
            ("Skool/G/a.mp4", 8.0)
        );
        let ev = done(&mut rx, 1).await;
        let file = ev[0].result.clone().unwrap();
        assert!(e.cache.join(&file).is_file() && file.ends_with(".jpg") && file.len() == 68);
        let again = svc.request(&e.root, &[item("Skool/G/a.mp4")], None);
        assert_eq!(again[0].state, ThumbState::Ready);
        assert_eq!(again[0].file.as_deref(), Some(file.as_str()));
        assert_eq!(runs(&e), 1);
        assert!(std::fs::read_dir(&e.cache)
            .unwrap()
            .flatten()
            .all(|f| !f.file_name().to_string_lossy().contains(".part")));
    }

    #[tokio::test]
    async fn ffmpeg_gets_plain_arguments_and_a_width_and_time_clamped_request() {
        let e2 = env(WRITE);
        let (svc, mut rx) = service(cfg(&e2));
        svc.request(
            &e2.root,
            &[ThumbItem {
                path: "Skool/G/a.mp4".into(),
                at_secs: Some(12.5),
            }],
            Some(320),
        );
        done(&mut rx, 1).await;
        let args_file = format!("{}.args", e2.log.display());
        let args: Vec<String> = std::fs::read_to_string(&args_file)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        let head: Vec<&str> = args.iter().take(8).map(String::as_str).collect();
        assert_eq!(
            head,
            [
                "-nostdin",
                "-hide_banner",
                "-loglevel",
                "error",
                "-ss",
                "12.5",
                "-i",
                &*e2.root.join("Skool/G/a.mp4").to_string_lossy()
            ]
        );
        assert_eq!(&args[8..12], ["-frames:v", "1", "-vf", "scale=320:-2"]);
        assert_eq!(&args[12..14], ["-q:v", "5"]);
        assert!(args[14].ends_with(".part.jpg"), "{}", args[14]);
        let odd = svc.request(
            &e2.root,
            &[
                ThumbItem {
                    path: "Skool/G/b.mp4".into(),
                    at_secs: Some(f64::NAN),
                },
                ThumbItem {
                    path: "Skool/G/c.mp4".into(),
                    at_secs: Some(-3.0),
                },
            ],
            Some(99999),
        );
        assert_eq!((odd[0].at_secs, odd[1].at_secs), (8.0, 0.0));
    }

    #[tokio::test]
    async fn a_changed_file_invalidates_its_thumbnail() {
        let e = env(WRITE);
        let (svc, mut rx) = service(cfg(&e));
        svc.request(&e.root, &[item("Skool/G/a.mp4")], None);
        done(&mut rx, 1).await;
        let f = std::fs::File::options()
            .write(true)
            .open(e.root.join("Skool/G/a.mp4"))
            .unwrap();
        f.set_modified(std::time::SystemTime::now() + Duration::from_secs(3600))
            .unwrap();
        let after = svc.request(&e.root, &[item("Skool/G/a.mp4")], None);
        assert_eq!(after[0].state, ThumbState::Pending);
        done(&mut rx, 1).await;
        assert_eq!(runs(&e), 2);
    }

    #[tokio::test]
    async fn the_same_request_in_flight_runs_ffmpeg_once() {
        let e = env("sleep 0.2\nprintf 'jpg' > \"$out\"");
        let (svc, mut rx) = service(cfg(&e));
        let a = svc.request(
            &e.root,
            &[item("Skool/G/a.mp4"), item("Skool/G/a.mp4")],
            None,
        );
        let b = svc.request(&e.root, &[item("Skool/G/a.mp4")], None);
        assert!(a.iter().chain(&b).all(|r| r.state == ThumbState::Pending));
        done(&mut rx, 1).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(runs(&e), 1);
        assert!(rx.try_recv().is_err(), "one event per thumbnail");
    }

    #[tokio::test]
    async fn at_most_two_ffmpeg_processes_run_at_once() {
        let e = env("sleep 0.2\nprintf 'jpg' > \"$out\"");
        let (svc, mut rx) = service(cfg(&e));
        let items: Vec<_> = ["a", "b", "c", "d", "e", "f"]
            .iter()
            .map(|n| item(&format!("Skool/G/{n}.mp4")))
            .collect();
        svc.request(&e.root, &items, None);
        let st = svc.status();
        assert!(st.running <= 2 && st.running + st.queued == 6, "{st:?}");
        done(&mut rx, 6).await;
        let mut now = 0i32;
        let mut peak = 0i32;
        for l in log_lines(&e) {
            if l.starts_with("start") {
                now += 1;
                peak = peak.max(now);
            } else if l == "end" {
                now -= 1;
            }
        }
        assert_eq!((peak, runs(&e)), (2, 6));
        let end = svc.status();
        assert_eq!((end.running, end.queued), (0, 0));
    }

    #[tokio::test]
    async fn the_newest_request_runs_first_and_cancel_drops_only_queued_items() {
        let e = env("sleep 0.15\nprintf 'jpg' > \"$out\"");
        let mut c = cfg(&e);
        c.workers = 1;
        let (svc, mut rx) = service(c);
        let names = ["a", "b", "c", "d"];
        for n in names {
            svc.request(&e.root, &[item(&format!("Skool/G/{n}.mp4"))], None);
        }
        done(&mut rx, 4).await;
        let order: Vec<String> = log_lines(&e)
            .iter()
            .filter(|l| l.starts_with("start"))
            .map(|l| {
                l.split(' ')
                    .nth(1)
                    .unwrap()
                    .rsplit('/')
                    .next()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(order, ["a.mp4", "d.mp4", "c.mp4", "b.mp4"]);

        let e = env("sleep 0.15\nprintf 'jpg' > \"$out\"");
        let mut c = cfg(&e);
        c.workers = 1;
        let (svc, mut rx) = service(c);
        for n in ["a", "b", "c"] {
            svc.request(&e.root, &[item(&format!("Skool/G/{n}.mp4"))], None);
        }
        svc.cancel(&[
            "Skool/G/b.mp4".to_string(),
            "Skool/G/c.mp4".to_string(),
            "Skool/G/a.mp4".to_string(),
        ]);
        done(&mut rx, 1).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(
            runs(&e),
            1,
            "the running one finishes, the queued ones are dropped"
        );
        let again = svc.request(&e.root, &[item("Skool/G/b.mp4")], None);
        assert_eq!(
            again[0].state,
            ThumbState::Pending,
            "a cancelled item can be requested again"
        );
    }

    #[tokio::test]
    async fn a_hanging_ffmpeg_is_killed_and_leaves_no_partial_file() {
        let e = env("printf 'half' > \"$out\"\nexec sleep 30");
        let mut c = cfg(&e);
        c.timeout = Duration::from_millis(300);
        let (svc, mut rx) = service(c);
        let t = std::time::Instant::now();
        svc.request(&e.root, &[item("Skool/G/a.mp4")], None);
        let ev = done(&mut rx, 1).await;
        assert_eq!(ev[0].result, Err("timeout".to_string()));
        assert!(t.elapsed() < Duration::from_secs(5));
        assert_eq!(std::fs::read_dir(&e.cache).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn failures_are_cached_briefly_so_they_do_not_retry_in_a_loop() {
        let e = env(FAIL);
        let mut c = cfg(&e);
        c.negative_ttl = Duration::from_millis(400);
        let (svc, mut rx) = service(c);
        svc.request(&e.root, &[item("Skool/G/a.mp4")], None);
        let ev = done(&mut rx, 1).await;
        assert_eq!(ev[0].result, Err("ffmpeg_failed".to_string()));
        let again = svc.request(&e.root, &[item("Skool/G/a.mp4")], None);
        assert_eq!(again[0].state, ThumbState::Unavailable);
        assert_eq!(again[0].reason.as_deref(), Some("ffmpeg_failed"));
        assert_eq!(runs(&e), 2, "one attempt at the requested time, one at 0");
        tokio::time::sleep(Duration::from_millis(500)).await;
        let later = svc.request(&e.root, &[item("Skool/G/a.mp4")], None);
        assert_eq!(later[0].state, ThumbState::Pending, "retried after the ttl");
        done(&mut rx, 1).await;
    }

    #[tokio::test]
    async fn a_file_shorter_than_the_time_is_retried_at_zero() {
        let e = env("if [ \"$ss\" = \"0\" ]; then printf 'jpg' > \"$out\"; fi");
        let (svc, mut rx) = service(cfg(&e));
        svc.request(&e.root, &[item("Skool/G/a.mp4")], None);
        let ev = done(&mut rx, 1).await;
        assert!(ev[0].result.is_ok(), "{:?}", ev[0].result);
        let starts: Vec<String> = log_lines(&e)
            .into_iter()
            .filter(|l| l.starts_with("start"))
            .collect();
        assert!(
            starts[0].ends_with(" 8") && starts[1].ends_with(" 0"),
            "{starts:?}"
        );
        let svc_at_zero = svc.request(
            &e.root,
            &[ThumbItem {
                path: "Skool/G/b.mp4".into(),
                at_secs: Some(0.0),
            }],
            None,
        );
        assert_eq!(svc_at_zero[0].state, ThumbState::Pending);
    }

    #[tokio::test]
    async fn missing_ffmpeg_is_unavailable_with_a_typed_reason() {
        let e = env(WRITE);
        let mut c = cfg(&e);
        c.ffmpeg = None;
        let (svc, _rx) = service(c);
        let r = svc.request(&e.root, &[item("Skool/G/a.mp4")], None);
        assert_eq!(
            (r[0].state, r[0].reason.as_deref()),
            (ThumbState::Unavailable, Some("ffmpeg_missing"))
        );
        let st = svc.status();
        assert!(!st.ffmpeg_available);
        let mut c = cfg(&e);
        c.ffmpeg = Some(e._tmp.path().join("nope"));
        let (svc, mut rx) = service(c);
        svc.request(&e.root, &[item("Skool/G/a.mp4")], None);
        assert_eq!(
            done(&mut rx, 1).await[0].result,
            Err("ffmpeg_missing".to_string())
        );
    }

    #[tokio::test]
    async fn paths_are_confined_to_regular_files_under_the_root() {
        let e = env(WRITE);
        std::fs::write(e._tmp.path().join("secret.mp4"), "s").unwrap();
        std::os::unix::fs::symlink(
            e._tmp.path().join("secret.mp4"),
            e.root.join("Skool/G/link.mp4"),
        )
        .unwrap();
        let (svc, _rx) = service(cfg(&e));
        for bad in [
            "../secret.mp4",
            "Skool/../../secret.mp4",
            "/etc/passwd",
            "Skool/G",
            "Skool/G/link.mp4",
            "Skool/G/missing.mp4",
            "",
            "Skool\\G\\a.mp4",
        ] {
            let r = svc.request(&e.root, &[item(bad)], None);
            assert_eq!(
                (r[0].state, r[0].reason.as_deref()),
                (ThumbState::Unavailable, Some("invalid_path")),
                "{bad:?}"
            );
        }
        assert_eq!(runs(&e), 0);
    }

    #[test]
    fn the_cache_cap_removes_the_oldest_files_first() {
        let tmp = tempfile::tempdir().unwrap();
        let now = std::time::SystemTime::now();
        for (i, name) in ["a", "b", "c", "d"].iter().enumerate() {
            let p = tmp.path().join(format!("{}.jpg", name.repeat(64)));
            std::fs::write(&p, vec![0u8; 100]).unwrap();
            let f = std::fs::File::options().write(true).open(&p).unwrap();
            f.set_modified(now - Duration::from_secs(1000 - i as u64 * 100))
                .unwrap();
        }
        std::fs::write(tmp.path().join("x.part.jpg"), "stale").unwrap();
        std::fs::write(tmp.path().join("notes.txt"), "keep").unwrap();
        assert_eq!(cache_bytes(tmp.path()), 400);
        enforce_cap(tmp.path(), 250);
        let mut left: Vec<String> = std::fs::read_dir(tmp.path())
            .unwrap()
            .flatten()
            .map(|f| f.file_name().to_string_lossy().chars().take(1).collect())
            .collect();
        left.sort();
        assert_eq!(
            left,
            ["c", "d", "n"]
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        );
        assert_eq!(cache_bytes(tmp.path()), 200);
    }

    #[tokio::test]
    async fn startup_cleans_an_oversized_cache_and_batches_enforce_the_cap_again() {
        let e = env(WRITE);
        std::fs::create_dir_all(&e.cache).unwrap();
        for i in 0..4 {
            std::fs::write(
                e.cache.join(format!("{}.jpg", format!("{i}").repeat(64))),
                vec![0u8; 100],
            )
            .unwrap();
        }
        let mut c = cfg(&e);
        c.cap_bytes = 150;
        let (svc, mut rx) = service(c);
        assert!(svc.status().cache_bytes <= 150);
        svc.request(&e.root, &[item("Skool/G/a.mp4")], None);
        done(&mut rx, 1).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            svc.status().cache_bytes <= 150,
            "{}",
            svc.status().cache_bytes
        );
    }

    /// Reads ONE video of `$LUMEN_SAMPLE_LIBRARY` (default `~/Vídeos/Lumen`)
    /// and writes only into a temp cache. Prints the time per thumbnail.
    #[tokio::test]
    #[ignore = "needs ffmpeg and a real library (LUMEN_SAMPLE_LIBRARY or ~/Vídeos/Lumen)"]
    async fn real_ffmpeg_makes_a_thumbnail_from_the_real_library() {
        fn first_mp4(dir: &Path, depth: usize) -> Option<PathBuf> {
            let mut entries: Vec<_> = std::fs::read_dir(dir).ok()?.flatten().collect();
            entries.sort_by_key(std::fs::DirEntry::file_name);
            for e in &entries {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "mp4") {
                    return Some(p);
                }
            }
            if depth == 0 {
                return None;
            }
            entries
                .iter()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .find_map(|e| first_mp4(&e.path(), depth - 1))
        }
        let root = std::env::var_os("LUMEN_SAMPLE_LIBRARY")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Vídeos/Lumen")))
            .unwrap();
        let video = first_mp4(&root, 6).expect("an mp4 in the library");
        let rel = video
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let ffmpeg = std::env::var_os("PATH")
            .into_iter()
            .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
            .map(|d| d.join("ffmpeg"))
            .find(|p| p.is_file())
            .expect("ffmpeg on PATH");
        let tmp = tempfile::tempdir().unwrap();
        let (svc, mut rx) = service(ThumbConfig::new(tmp.path().join("thumbs"), Some(ffmpeg)));
        let t = std::time::Instant::now();
        svc.request(&root, &[item(&rel)], None);
        let ev = done(&mut rx, 1).await;
        let file = ev[0].result.clone().unwrap();
        let bytes = std::fs::read(tmp.path().join("thumbs").join(&file)).unwrap();
        println!(
            "thumbnail in {} ms, {} bytes",
            t.elapsed().as_millis(),
            bytes.len()
        );
        assert!(bytes.starts_with(&[0xff, 0xd8]), "a JPEG");
    }
}
