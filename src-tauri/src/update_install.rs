//! In-place self-update: detects how the running app is installed, downloads
//! the matching asset of the newest release, verifies it (SHA-256 against
//! `SHA256SUMS.txt` and a minisign signature against the key built into the
//! app, failing closed) and installs it with the right tool for that install
//! kind, then restarts. The backend re-reads the release metadata itself and
//! only accepts exact asset names from GitHub hosts: nothing the UI sends is
//! trusted. Installing goes through the [`System`] trait, so every step is
//! tested with a fake; argv arrays only, never a shell string.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use lumen_core::CancelFlag;
use serde::Serialize;
use serde_json::json;

use crate::downloads::ErrorCode;
use crate::thumbs::Emit;
use crate::update::{status_error, Asset, UpdateFailure, UpdateService, Version};
use crate::update_verify::{verify_checksum, verify_signature, VerifyError};

pub const PROGRESS_EVENT: &str = "update://progress";
pub const SUMS_ASSET: &str = "SHA256SUMS.txt";
const PACKAGE_NAME: &str = "lumen";
const PACKAGE_RELAUNCH: &str = "/usr/bin/lumen";
const MAX_TEXT_ASSET_BYTES: usize = 1024 * 1024;
const STALL_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_REDIRECTS: usize = 5;
const MAX_STDERR_LINE: usize = 200;
/// pkexec exit codes: dialog dismissed (126) and authentication failed (127).
const POLKIT_DENIED: [i32; 2] = [126, 127];

// ---- install kind ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    Linux,
    Other,
}

impl Os {
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else {
            Self::Other
        }
    }
}

/// How the running app got onto the machine, which decides how it updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum InstallKind {
    #[serde(rename = "windows-nsis")]
    WindowsNsis,
    #[serde(rename = "appimage")]
    Appimage,
    #[serde(rename = "rpm")]
    Rpm,
    #[serde(rename = "deb")]
    Deb,
    /// Anything else (a binary under `$HOME`...): updated by hand.
    #[serde(rename = "unmanaged")]
    Unmanaged,
}

/// Asks the package managers who owns a file (faked in tests).
pub trait PackageProbe: Send + Sync {
    fn rpm_owner(&self, exe: &Path) -> Option<String>;
    fn dpkg_owner(&self, exe: &Path) -> Option<String>;
}

pub struct KindEnv<'a> {
    pub os: Os,
    pub exe: &'a Path,
    /// `$APPIMAGE`, when set.
    pub appimage: Option<&'a Path>,
    pub probe: &'a dyn PackageProbe,
}

pub fn detect_kind(env: &KindEnv<'_>) -> InstallKind {
    match env.os {
        Os::Windows => InstallKind::WindowsNsis,
        Os::Other => InstallKind::Unmanaged,
        Os::Linux => {
            if env.appimage.is_some_and(Path::is_file) {
                InstallKind::Appimage
            } else if env.exe.starts_with("/usr") {
                if env
                    .probe
                    .rpm_owner(env.exe)
                    .is_some_and(|n| n == PACKAGE_NAME)
                {
                    InstallKind::Rpm
                } else if env
                    .probe
                    .dpkg_owner(env.exe)
                    .is_some_and(|n| n == PACKAGE_NAME)
                {
                    InstallKind::Deb
                } else {
                    InstallKind::Unmanaged
                }
            } else {
                InstallKind::Unmanaged
            }
        }
    }
}

/// The real probe: `rpm -qf` and `dpkg -S`.
pub struct SystemProbe;

fn first_stdout(program: &str, args: &[&std::ffi::OsStr]) -> Option<String> {
    let out = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    String::from_utf8(out.stdout)
        .ok()
        .map(|s| s.trim().to_string())
}

impl PackageProbe for SystemProbe {
    fn rpm_owner(&self, exe: &Path) -> Option<String> {
        first_stdout(
            "rpm",
            &[
                "-qf".as_ref(),
                "--qf".as_ref(),
                "%{NAME}".as_ref(),
                exe.as_os_str(),
            ],
        )
    }

    fn dpkg_owner(&self, exe: &Path) -> Option<String> {
        let line = first_stdout("dpkg", &["-S".as_ref(), exe.as_os_str()])?;
        line.lines().next()?.split(':').next().map(str::to_string)
    }
}

/// Exact name of the installer for `kind` in release `version` (what the
/// Tauri bundler produces), or `None` for kinds without one.
pub fn expected_asset(kind: InstallKind, version: &Version, arch: &str) -> Option<String> {
    let v = version.to_string();
    let (deb, appimage, win) = match arch {
        "x86_64" => ("amd64", "amd64", "x64"),
        "aarch64" => ("arm64", "aarch64", "arm64"),
        _ => return None,
    };
    Some(match kind {
        InstallKind::Rpm => format!("Lumen-{v}-1.{arch}.rpm"),
        InstallKind::Deb => format!("Lumen_{v}_{deb}.deb"),
        InstallKind::Appimage => format!("Lumen_{v}_{appimage}.AppImage"),
        InstallKind::WindowsNsis => format!("Lumen_{v}_{win}-setup.exe"),
        InstallKind::Unmanaged => return None,
    })
}

// ---- argv (fixed arrays, never a shell) ----

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| (*s).to_string()).collect()
}

/// The command that installs `file`, or `None` when the kind has none or no
/// package manager is available.
pub fn install_argv(
    kind: InstallKind,
    file: &Path,
    has: &dyn Fn(&str) -> bool,
) -> Option<Vec<String>> {
    let f = file.to_string_lossy().into_owned();
    match kind {
        InstallKind::Rpm => ["dnf", "dnf5"]
            .into_iter()
            .find(|m| has(m))
            .map(|m| argv(&["pkexec", m, "install", "-y", &f]))
            .or_else(|| has("rpm").then(|| argv(&["pkexec", "rpm", "-U", "--replacepkgs", &f]))),
        InstallKind::Deb => {
            has("apt-get").then(|| argv(&["pkexec", "apt-get", "install", "-y", &f]))
        }
        // The flags tauri-plugin-updater passes to the NSIS installer:
        // passive UI (/P), update mode (/UPDATE) and relaunch (/R).
        InstallKind::WindowsNsis => Some(argv(&[&f, "/P", "/UPDATE", "/R"])),
        InstallKind::Appimage | InstallKind::Unmanaged => None,
    }
}

/// Asks the package manager which version of lumen is installed now.
pub fn version_argv(kind: InstallKind) -> Option<Vec<String>> {
    match kind {
        InstallKind::Rpm => Some(argv(&["rpm", "-q", "--qf", "%{VERSION}", PACKAGE_NAME])),
        InstallKind::Deb => Some(argv(&["dpkg-query", "-W", "-f=${Version}", PACKAGE_NAME])),
        _ => None,
    }
}

/// Starts the new binary detached. Windows relaunches through the installer.
pub fn relaunch_argv(kind: InstallKind, appimage: Option<&Path>) -> Option<Vec<String>> {
    match kind {
        InstallKind::Rpm | InstallKind::Deb => Some(argv(&["setsid", "-f", PACKAGE_RELAUNCH])),
        InstallKind::Appimage => appimage.map(|p| argv(&["setsid", "-f", &p.to_string_lossy()])),
        InstallKind::WindowsNsis | InstallKind::Unmanaged => None,
    }
}

/// Package managers report `0.4.0`, `0.4.0~rc.1` or `0.4.0-1`; compare with
/// the release version without being fooled by prefixes.
pub fn same_version(installed: &str, expected: &str) -> bool {
    let installed = installed.trim().replace('~', "-");
    if installed.is_empty() {
        return false;
    }
    installed == expected
        || installed
            .strip_prefix(&format!("{expected}-"))
            .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
}

// ---- download policy, throttle, cache ----

/// Hosts a download may come from (and every redirect hop).
#[derive(Debug, Clone)]
pub struct DownloadPolicy {
    hosts: Vec<String>,
    require_https: bool,
}

impl DownloadPolicy {
    /// GitHub only, over HTTPS. `api.github.com` is needed because private
    /// repositories serve assets through the API URL.
    pub fn github() -> Self {
        Self {
            hosts: [
                "github.com",
                "objects.githubusercontent.com",
                "release-assets.githubusercontent.com",
                "api.github.com",
            ]
            .iter()
            .map(|h| (*h).to_string())
            .collect(),
            require_https: true,
        }
    }

    #[cfg(test)]
    pub fn loopback() -> Self {
        Self {
            hosts: vec!["localhost".into()],
            require_https: false,
        }
    }

    pub fn allows(&self, url: &url::Url) -> bool {
        let scheme_ok = url.scheme() == "https" || (!self.require_https && url.scheme() == "http");
        scheme_ok
            && url.username().is_empty()
            && url
                .host_str()
                .is_some_and(|h| self.hosts.iter().any(|a| a == h))
    }
}

pub struct Throttle {
    interval: Duration,
    last: Option<Instant>,
}

impl Throttle {
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            last: None,
        }
    }

    /// True when an event may go out now; `force` (the last one) always may.
    pub fn ready(&mut self, now: Instant, force: bool) -> bool {
        let due = force
            || self
                .last
                .is_none_or(|l| now.duration_since(l) >= self.interval);
        if due {
            self.last = Some(now);
        }
        due
    }
}

/// Removes everything under the updates folder (nothing is in flight at startup).
pub fn clean_stale(updates_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(updates_dir) else {
        return;
    };
    for e in entries.flatten() {
        let path = e.path();
        let _ = if e.file_type().is_ok_and(|t| t.is_dir()) {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
    }
}

fn private_dir(path: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(path)
}

fn private_file(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)
}

// ---- running things ----

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutput {
    pub code: Option<i32>,
    pub stdout: String,
    /// Last non-empty stderr line, capped.
    pub stderr_last: String,
}

/// The outside world the installer touches (faked in tests).
#[async_trait]
pub trait System: Send + Sync {
    /// Runs `argv` and waits. `Err` means it could not even start.
    async fn run(&self, argv: &[String]) -> Result<RunOutput, String>;
    fn spawn_detached(&self, argv: &[String]) -> Result<(), String>;
    fn has_command(&self, name: &str) -> bool;
    /// Ends the app (after the new version was started or the installer launched).
    fn exit(&self);
}

pub struct ProcessSystem {
    exit: Arc<dyn Fn() + Send + Sync>,
}

impl ProcessSystem {
    pub fn new(exit: Arc<dyn Fn() + Send + Sync>) -> Self {
        Self { exit }
    }
}

fn last_line(text: &str) -> String {
    let line = text
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    line.chars().take(MAX_STDERR_LINE).collect()
}

#[async_trait]
impl System for ProcessSystem {
    async fn run(&self, argv: &[String]) -> Result<RunOutput, String> {
        let (program, args) = argv.split_first().ok_or("empty command")?;
        let out = tokio::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|e| format!("cannot start {program}: {e}"))?;
        Ok(RunOutput {
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).trim().to_string(),
            stderr_last: last_line(&String::from_utf8_lossy(&out.stderr)),
        })
    }

    fn spawn_detached(&self, argv: &[String]) -> Result<(), String> {
        let (program, args) = argv.split_first().ok_or("empty command")?;
        let mut cmd = std::process::Command::new(program);
        cmd.args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        #[cfg(windows)]
        // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
        std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x0000_0008 | 0x0000_0200);
        cmd.spawn()
            .map(|_| ())
            .map_err(|e| format!("cannot start {program}: {e}"))
    }

    fn has_command(&self, name: &str) -> bool {
        std::env::var_os("PATH")
            .into_iter()
            .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
            .any(|d| d.join(name).is_file())
    }

    fn exit(&self) {
        (self.exit)();
    }
}

/// What the updater needs to know about this installation and machine.
#[derive(Clone)]
pub struct UpdateEnv {
    pub kind: InstallKind,
    /// `std::env::consts::ARCH` style: `x86_64`, `aarch64`.
    pub arch: String,
    /// `$APPIMAGE`, when set.
    pub appimage: Option<PathBuf>,
    /// `<app cache dir>/updates`.
    pub updates_dir: PathBuf,
    pub system: Arc<dyn System>,
    pub emit: Emit,
}

#[cfg(test)]
struct NoSystem;

#[cfg(test)]
#[async_trait]
impl System for NoSystem {
    async fn run(&self, _: &[String]) -> Result<RunOutput, String> {
        Err("not available in this test".into())
    }
    fn spawn_detached(&self, _: &[String]) -> Result<(), String> {
        Err("not available in this test".into())
    }
    fn has_command(&self, _: &str) -> bool {
        false
    }
    fn exit(&self) {}
}

impl UpdateEnv {
    /// An environment that cannot run or emit anything (check-only tests).
    #[cfg(test)]
    pub fn for_tests(kind: InstallKind) -> Self {
        Self {
            kind,
            arch: "x86_64".into(),
            appimage: None,
            updates_dir: std::env::temp_dir().join("lumen-update-tests-unused"),
            system: Arc::new(NoSystem),
            emit: Arc::new(|_, _| {}),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallOutcome {
    pub version: String,
}

/// Frees the install slot however the run ends.
struct Slot<'a>(&'a std::sync::Mutex<Option<CancelFlag>>);

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

fn fail(code: ErrorCode, message: impl Into<String>) -> UpdateFailure {
    UpdateFailure::new(code, message)
}

fn verify_failure(e: VerifyError) -> UpdateFailure {
    match e {
        VerifyError::ChecksumMismatch(m) => fail(ErrorCode::ChecksumMismatch, m),
        VerifyError::SignatureInvalid(m) => fail(ErrorCode::SignatureInvalid, m),
        VerifyError::Io(m) => fail(ErrorCode::Disk, m),
    }
}

/// Copies `file` next to `target` and renames it over it, so the running
/// AppImage is replaced atomically.
fn replace_appimage(file: &Path, target: &Path) -> std::io::Result<()> {
    let mut staged = target.as_os_str().to_owned();
    staged.push(".new");
    let staged = PathBuf::from(staged);
    let result = (|| {
        std::fs::copy(file, &staged)?;
        #[cfg(unix)]
        std::fs::set_permissions(&staged, std::os::unix::fs::PermissionsExt::from_mode(0o755))?;
        std::fs::rename(&staged, target)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&staged);
    }
    result
}

impl UpdateService {
    /// Adjusts download hosts, the verification key and the progress rate
    /// (tests use a loopback policy, a throwaway key and no throttling).
    #[cfg(test)]
    #[must_use]
    pub fn with_install_options(
        mut self,
        policy: DownloadPolicy,
        pubkey: &str,
        interval: Duration,
    ) -> Self {
        self.policy = policy;
        self.pubkey = pubkey.to_string();
        self.progress_interval = interval;
        self
    }

    #[cfg(test)]
    pub fn core_for_tests(&self) -> Arc<crate::state::AppCore> {
        Arc::clone(&self.core)
    }

    #[cfg(test)]
    pub fn env_for_tests(&self) -> UpdateEnv {
        self.env.clone()
    }

    /// Cancels a running install; false when none is running.
    pub fn cancel(&self) -> bool {
        match self
            .running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            Some(flag) => {
                flag.cancel();
                true
            }
            None => false,
        }
    }

    fn emit_progress(&self, stage: &str, downloaded: u64, total: u64) {
        (self.env.emit)(
            PROGRESS_EVENT,
            json!({"stage": stage, "downloaded": downloaded, "total": total}),
        );
    }

    /// Downloads one asset, feeding every chunk to `sink`; returns the byte
    /// count. Credentials only go to the API host, never to a redirect hop.
    async fn fetch(
        &self,
        asset: &Asset,
        token: Option<&str>,
        cancel: &CancelFlag,
        mut sink: impl FnMut(&[u8]) -> std::io::Result<()>,
        mut on_bytes: impl FnMut(u64),
    ) -> Result<u64, UpdateFailure> {
        let raw = if token.is_some() && !asset.url.is_empty() {
            &asset.url
        } else {
            &asset.download_url
        };
        let url = url::Url::parse(raw)
            .ok()
            .filter(|u| self.policy.allows(u))
            .ok_or_else(|| {
                fail(
                    ErrorCode::InvalidResponse,
                    "the download is not served from an allowed host",
                )
            })?;
        let policy = self.policy.clone();
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::custom(move |attempt| {
                if attempt.previous().len() >= MAX_REDIRECTS || !policy.allows(attempt.url()) {
                    attempt.error("redirect to a host that is not allowed")
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .map_err(|e| fail(ErrorCode::Internal, e.without_url().to_string()))?;
        let mut req = client
            .get(url.clone())
            .header(reqwest::header::ACCEPT, "application/octet-stream")
            .header(
                reqwest::header::USER_AGENT,
                format!("lumen/{}", self.current),
            );
        let api_host = url::Url::parse(&self.base)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string));
        if let (Some(token), true) = (token, url.host_str().map(str::to_string) == api_host) {
            let mut v = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|_| fail(ErrorCode::InvalidToken, "the saved token cannot be sent"))?;
            v.set_sensitive(true);
            req = req.header(reqwest::header::AUTHORIZATION, v);
        }
        let mut resp = req.send().await.map_err(|e| {
            if e.is_redirect() {
                fail(
                    ErrorCode::InvalidResponse,
                    "the download redirected to a host that is not allowed",
                )
            } else {
                fail(
                    ErrorCode::Network,
                    format!("cannot download the update: {}", e.without_url()),
                )
            }
        })?;
        if resp.status().as_u16() != 200 {
            let e = status_error(resp.status().as_u16(), resp.headers(), token.is_some());
            return Err(UpdateFailure::from(e));
        }
        let mut total = 0u64;
        loop {
            if cancel.is_cancelled() {
                return Err(fail(ErrorCode::Cancelled, "the update was cancelled"));
            }
            let chunk = tokio::time::timeout(STALL_TIMEOUT, resp.chunk())
                .await
                .map_err(|_| fail(ErrorCode::Network, "the download stalled"))?
                .map_err(|e| {
                    fail(
                        ErrorCode::Network,
                        format!("the download was interrupted: {}", e.without_url()),
                    )
                })?;
            let Some(chunk) = chunk else { break };
            sink(&chunk)
                .map_err(|e| fail(ErrorCode::Disk, format!("cannot write the update: {e}")))?;
            total += chunk.len() as u64;
            on_bytes(total);
        }
        Ok(total)
    }

    async fn fetch_text(
        &self,
        asset: &Asset,
        token: Option<&str>,
        cancel: &CancelFlag,
    ) -> Result<String, UpdateFailure> {
        let mut buf: Vec<u8> = Vec::new();
        let mut too_big = false;
        self.fetch(
            asset,
            token,
            cancel,
            |c| {
                if buf.len() + c.len() > MAX_TEXT_ASSET_BYTES {
                    too_big = true;
                    return Err(std::io::Error::other("too large"));
                }
                buf.extend_from_slice(c);
                Ok(())
            },
            |_| {},
        )
        .await
        .map_err(|e| {
            if too_big {
                fail(
                    ErrorCode::InvalidResponse,
                    "a release file is unexpectedly large",
                )
            } else {
                e
            }
        })?;
        String::from_utf8(buf)
            .map_err(|_| fail(ErrorCode::InvalidResponse, "a release file is not text"))
    }

    /// Downloads, verifies, installs and restarts. Returns when the install
    /// finished or failed (a successful run ends the app first).
    pub async fn install(&self) -> Result<InstallOutcome, UpdateFailure> {
        let cancel = CancelFlag::new();
        {
            let mut running = self.running.lock().unwrap_or_else(|e| e.into_inner());
            if running.is_some() {
                return Err(fail(
                    ErrorCode::AlreadyRunning,
                    "an update is already running",
                ));
            }
            *running = Some(cancel.clone());
        }
        let _slot = Slot(&self.running);
        let token = self.load_token()?;
        let current = Version::parse(&self.current).ok_or_else(|| {
            fail(
                ErrorCode::Internal,
                "the app version is not a valid version",
            )
        })?;
        let include = self.settings()?.include_prereleases;
        let releases = self.fetch_listing(token.as_deref()).await?;
        let release = crate::update::select(&releases, include).ok_or_else(|| {
            fail(
                ErrorCode::NoReleases,
                "the repository has no usable release",
            )
        })?;
        let latest = Version::parse(&release.tag_name).ok_or_else(|| {
            fail(
                ErrorCode::InvalidResponse,
                "the release tag is not a version",
            )
        })?;
        if latest <= current {
            return Err(fail(
                ErrorCode::UpToDate,
                "this is already the newest version",
            ));
        }
        let assets = release.assets.clone().unwrap_or_default();
        let dir = self.env.updates_dir.join(latest.to_string());
        let result = self
            .process(&latest, &assets, token.as_deref(), &cancel, &dir)
            .await;
        match &result {
            Err(e) if e.code == ErrorCode::ManualInstallRequired => {}
            Err(_) => {
                let _ = std::fs::remove_dir_all(&dir);
            }
            Ok(_) => {}
        }
        result
    }

    async fn process(
        &self,
        latest: &Version,
        assets: &[Asset],
        token: Option<&str>,
        cancel: &CancelFlag,
        dir: &Path,
    ) -> Result<InstallOutcome, UpdateFailure> {
        let kind = self.env.kind;
        let name_kind = if kind == InstallKind::Unmanaged {
            InstallKind::Appimage
        } else {
            kind
        };
        let name = expected_asset(name_kind, latest, &self.env.arch)
            .ok_or_else(|| fail(ErrorCode::NoReleases, "no installer exists for this system"))?;
        let find = |n: &str| assets.iter().find(|a| a.name == n);
        let installer = find(&name)
            .ok_or_else(|| fail(ErrorCode::NoReleases, format!("the release has no {name}")))?;
        let sums = find(SUMS_ASSET).ok_or_else(|| {
            fail(
                ErrorCode::ChecksumMismatch,
                format!("the release has no {SUMS_ASSET}"),
            )
        })?;
        let sig = find(&format!("{name}.sig")).ok_or_else(|| {
            fail(
                ErrorCode::SignatureMissing,
                "the release is not signed (no .sig for the installer)",
            )
        })?;

        private_dir(dir).map_err(|e| {
            fail(
                ErrorCode::Disk,
                format!("cannot create the download folder: {e}"),
            )
        })?;
        let total = installer.size;
        self.emit_progress("downloading", 0, total);
        let sums_text = self.fetch_text(sums, token, cancel).await?;
        let sig_text = self.fetch_text(sig, token, cancel).await?;
        let file_path = dir.join(&name);
        let mut file = private_file(&file_path)
            .map_err(|e| fail(ErrorCode::Disk, format!("cannot create the download: {e}")))?;
        let mut throttle = Throttle::new(self.progress_interval);
        let written = {
            use std::io::Write;
            self.fetch(
                installer,
                token,
                cancel,
                |c| file.write_all(c),
                |n| {
                    if throttle.ready(Instant::now(), total > 0 && n >= total) {
                        self.emit_progress("downloading", n, total);
                    }
                },
            )
            .await?
        };
        drop(file);
        if total > 0 && written != total {
            return Err(fail(
                ErrorCode::ChecksumMismatch,
                "the downloaded size differs from the release metadata",
            ));
        }

        self.emit_progress("verifying", written, total);
        let (path, pubkey, name_owned) = (file_path.clone(), self.pubkey.clone(), name.clone());
        tokio::task::spawn_blocking(move || {
            verify_checksum(&path, &name_owned, &sums_text).map_err(verify_failure)?;
            verify_signature(&pubkey, sig_text.trim(), &path).map_err(verify_failure)
        })
        .await
        .map_err(|e| fail(ErrorCode::Internal, e.to_string()))??;

        self.install_verified(latest, &file_path, dir).await
    }

    async fn install_verified(
        &self,
        latest: &Version,
        file: &Path,
        dir: &Path,
    ) -> Result<InstallOutcome, UpdateFailure> {
        let kind = self.env.kind;
        let sys = &self.env.system;
        let outcome = InstallOutcome {
            version: latest.to_string(),
        };
        self.emit_progress("installing", 0, 0);
        match kind {
            InstallKind::Unmanaged => {
                let mut e = fail(
                    ErrorCode::ManualInstallRequired,
                    "this installation cannot be updated automatically: the verified file is ready to install by hand",
                );
                e.path = Some(file.to_string_lossy().into_owned());
                Err(e)
            }
            InstallKind::WindowsNsis => {
                let argv = install_argv(kind, file, &|_| false)
                    .ok_or_else(|| fail(ErrorCode::InstallFailed, "no installer command"))?;
                sys.spawn_detached(&argv)
                    .map_err(|m| fail(ErrorCode::InstallFailed, m))?;
                self.emit_progress("restarting", 0, 0);
                sys.exit();
                Ok(outcome)
            }
            InstallKind::Appimage => {
                let target = self.env.appimage.clone().ok_or_else(|| {
                    fail(
                        ErrorCode::InstallFailed,
                        "the running AppImage path is unknown",
                    )
                })?;
                replace_appimage(file, &target).map_err(|e| {
                    fail(ErrorCode::Disk, format!("cannot replace the AppImage: {e}"))
                })?;
                self.restart(kind, dir).await?;
                Ok(outcome)
            }
            InstallKind::Rpm | InstallKind::Deb => {
                let argv = install_argv(kind, file, &|n| sys.has_command(n)).ok_or_else(|| {
                    fail(ErrorCode::InstallFailed, "no package manager was found")
                })?;
                let out = sys
                    .run(&argv)
                    .await
                    .map_err(|m| fail(ErrorCode::InstallFailed, m))?;
                match out.code {
                    Some(0) => {}
                    Some(c) if POLKIT_DENIED.contains(&c) => {
                        return Err(fail(
                            ErrorCode::InstallCancelled,
                            "the installation was not authorized",
                        ))
                    }
                    _ => {
                        return Err(fail(
                            ErrorCode::InstallFailed,
                            format!("the installer failed: {}", out.stderr_last),
                        ))
                    }
                }
                let probe = version_argv(kind).ok_or_else(|| {
                    fail(
                        ErrorCode::InstallFailed,
                        "cannot check the installed version",
                    )
                })?;
                let installed = sys
                    .run(&probe)
                    .await
                    .map_err(|m| fail(ErrorCode::InstallFailed, m))?;
                if installed.code != Some(0)
                    || !same_version(&installed.stdout, &latest.to_string())
                {
                    return Err(fail(
                        ErrorCode::InstallFailed,
                        format!(
                            "the installed version is {} instead of {latest}",
                            installed.stdout
                        ),
                    ));
                }
                self.restart(kind, dir).await?;
                Ok(outcome)
            }
        }
    }

    /// Starts the new binary detached, then ends this process.
    async fn restart(&self, kind: InstallKind, dir: &Path) -> Result<(), UpdateFailure> {
        self.emit_progress("restarting", 0, 0);
        let _ = std::fs::remove_dir_all(dir);
        let argv = relaunch_argv(kind, self.env.appimage.as_deref())
            .ok_or_else(|| fail(ErrorCode::InstallFailed, "cannot restart the app"))?;
        self.env.system.spawn_detached(&argv).map_err(|m| {
            fail(
                ErrorCode::InstallFailed,
                format!("installed, but could not restart: {m}"),
            )
        })?;
        self.env.system.exit();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::{UpdateFailure, UpdateService};
    use lumen_download::testsrv::{Recorded, Reply, TestServer};
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    const TOKEN: &str = "ghp_SECRETTOKEN1234567890";
    const TEST_PUB: &str = include_str!("../tests/fixtures/update/test.pub");
    const OTHER_PUB: &str = include_str!("../tests/fixtures/update/other.pub");
    const SIG: &str = include_str!("../tests/fixtures/update/payload.bin.sig");

    fn payload() -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/update/payload.bin"),
        )
        .unwrap()
    }

    fn sha(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    // ---- install kind ----

    struct Probe(Option<&'static str>, Option<&'static str>);

    impl PackageProbe for Probe {
        fn rpm_owner(&self, _: &std::path::Path) -> Option<String> {
            self.0.map(str::to_string)
        }
        fn dpkg_owner(&self, _: &std::path::Path) -> Option<String> {
            self.1.map(str::to_string)
        }
    }

    #[test]
    fn the_install_kind_follows_how_the_running_binary_got_there() {
        let tmp = tempfile::tempdir().unwrap();
        let app = tmp.path().join("Lumen.AppImage");
        std::fs::write(&app, "x").unwrap();
        let kind = |os, exe: &str, appimage: Option<&std::path::Path>, probe: &Probe| {
            detect_kind(&KindEnv {
                os,
                exe: std::path::Path::new(exe),
                appimage,
                probe,
            })
        };
        let none = Probe(None, None);
        assert_eq!(
            kind(
                Os::Windows,
                "C:\\Users\\a\\AppData\\Local\\Lumen\\lumen.exe",
                None,
                &none
            ),
            InstallKind::WindowsNsis
        );
        assert_eq!(
            kind(
                Os::Linux,
                "/tmp/.mount_x/usr/bin/lumen",
                Some(&app),
                &Probe(Some("lumen"), None)
            ),
            InstallKind::Appimage
        );
        let missing = tmp.path().join("gone.AppImage");
        assert_eq!(
            kind(Os::Linux, "/home/u/bin/lumen", Some(&missing), &none),
            InstallKind::Unmanaged,
            "APPIMAGE pointing nowhere is not an AppImage"
        );
        assert_eq!(
            kind(
                Os::Linux,
                "/usr/bin/lumen",
                None,
                &Probe(Some("lumen"), None)
            ),
            InstallKind::Rpm
        );
        assert_eq!(
            kind(
                Os::Linux,
                "/usr/bin/lumen",
                None,
                &Probe(None, Some("lumen"))
            ),
            InstallKind::Deb
        );
        assert_eq!(
            kind(
                Os::Linux,
                "/usr/bin/lumen",
                None,
                &Probe(Some("other"), None)
            ),
            InstallKind::Unmanaged
        );
        assert_eq!(
            kind(Os::Linux, "/usr/bin/lumen", None, &none),
            InstallKind::Unmanaged
        );
        assert_eq!(
            kind(
                Os::Linux,
                "/home/u/.local/bin/lumen",
                None,
                &Probe(Some("lumen"), Some("lumen"))
            ),
            InstallKind::Unmanaged,
            "only /usr counts as a package install"
        );
        assert_eq!(
            kind(
                Os::Linux,
                "/usr/local/bin/lumen",
                None,
                &Probe(Some("lumen"), None)
            ),
            InstallKind::Rpm
        );
        assert_eq!(
            kind(Os::Other, "/Applications/Lumen", None, &none),
            InstallKind::Unmanaged
        );
        assert_eq!(
            serde_json::to_value([
                InstallKind::WindowsNsis,
                InstallKind::Appimage,
                InstallKind::Rpm,
                InstallKind::Deb,
                InstallKind::Unmanaged
            ])
            .unwrap(),
            json!(["windows-nsis", "appimage", "rpm", "deb", "unmanaged"])
        );
    }

    #[test]
    fn expected_asset_names_are_exact_per_kind_and_cpu() {
        let v = crate::update::Version::parse("0.4.0").unwrap();
        let name = |k, arch| expected_asset(k, &v, arch);
        assert_eq!(
            name(InstallKind::Rpm, "x86_64").as_deref(),
            Some("Lumen-0.4.0-1.x86_64.rpm")
        );
        assert_eq!(
            name(InstallKind::Rpm, "aarch64").as_deref(),
            Some("Lumen-0.4.0-1.aarch64.rpm")
        );
        assert_eq!(
            name(InstallKind::Deb, "x86_64").as_deref(),
            Some("Lumen_0.4.0_amd64.deb")
        );
        assert_eq!(
            name(InstallKind::Deb, "aarch64").as_deref(),
            Some("Lumen_0.4.0_arm64.deb")
        );
        assert_eq!(
            name(InstallKind::Appimage, "x86_64").as_deref(),
            Some("Lumen_0.4.0_amd64.AppImage")
        );
        assert_eq!(
            name(InstallKind::Appimage, "aarch64").as_deref(),
            Some("Lumen_0.4.0_aarch64.AppImage")
        );
        assert_eq!(
            name(InstallKind::WindowsNsis, "x86_64").as_deref(),
            Some("Lumen_0.4.0_x64-setup.exe")
        );
        assert_eq!(
            name(InstallKind::WindowsNsis, "aarch64").as_deref(),
            Some("Lumen_0.4.0_arm64-setup.exe")
        );
        assert_eq!(name(InstallKind::Unmanaged, "x86_64"), None);
        assert_eq!(
            name(InstallKind::Rpm, "riscv64"),
            None,
            "unknown CPUs get nothing"
        );
        let rc = crate::update::Version::parse("v0.4.0-rc.1").unwrap();
        assert_eq!(
            expected_asset(InstallKind::Deb, &rc, "x86_64").as_deref(),
            Some("Lumen_0.4.0-rc.1_amd64.deb")
        );
    }

    #[test]
    fn argv_vectors_are_fixed_arrays_without_a_shell() {
        let file =
            std::path::Path::new("/home/u/.cache/lumen/updates/0.4.0/Lumen-0.4.0-1.x86_64.rpm");
        let f = file.to_string_lossy().to_string();
        let has = |names: &'static [&'static str]| move |n: &str| names.contains(&n);
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            install_argv(InstallKind::Rpm, file, &has(&["dnf", "dnf5", "rpm"])),
            Some(v(&["pkexec", "dnf", "install", "-y", &f]))
        );
        assert_eq!(
            install_argv(InstallKind::Rpm, file, &has(&["dnf5", "rpm"])),
            Some(v(&["pkexec", "dnf5", "install", "-y", &f]))
        );
        assert_eq!(
            install_argv(InstallKind::Rpm, file, &has(&["rpm"])),
            Some(v(&["pkexec", "rpm", "-U", "--replacepkgs", &f]))
        );
        assert_eq!(install_argv(InstallKind::Rpm, file, &has(&[])), None);
        assert_eq!(
            install_argv(InstallKind::Deb, file, &has(&["apt-get"])),
            Some(v(&["pkexec", "apt-get", "install", "-y", &f]))
        );
        assert_eq!(
            install_argv(InstallKind::WindowsNsis, file, &has(&[])),
            Some(v(&[&f, "/P", "/UPDATE", "/R"]))
        );
        assert_eq!(install_argv(InstallKind::Appimage, file, &has(&[])), None);
        assert_eq!(install_argv(InstallKind::Unmanaged, file, &has(&[])), None);
        assert_eq!(
            version_argv(InstallKind::Rpm),
            Some(v(&["rpm", "-q", "--qf", "%{VERSION}", "lumen"]))
        );
        assert_eq!(
            version_argv(InstallKind::Deb),
            Some(v(&["dpkg-query", "-W", "-f=${Version}", "lumen"]))
        );
        assert_eq!(version_argv(InstallKind::Appimage), None);
        let app = std::path::Path::new("/opt/Lumen.AppImage");
        assert_eq!(
            relaunch_argv(InstallKind::Rpm, Some(app)),
            Some(v(&["setsid", "-f", "/usr/bin/lumen"]))
        );
        assert_eq!(
            relaunch_argv(InstallKind::Deb, None),
            Some(v(&["setsid", "-f", "/usr/bin/lumen"]))
        );
        assert_eq!(
            relaunch_argv(InstallKind::Appimage, Some(app)),
            Some(v(&["setsid", "-f", "/opt/Lumen.AppImage"]))
        );
        assert_eq!(relaunch_argv(InstallKind::Appimage, None), None);
        assert_eq!(
            relaunch_argv(InstallKind::WindowsNsis, None),
            None,
            "the installer relaunches with /R"
        );
    }

    #[test]
    fn versions_reported_by_the_package_manager_compare_loosely() {
        assert!(same_version("0.4.0", "0.4.0"));
        assert!(same_version("0.4.0~rc.1", "0.4.0-rc.1"));
        assert!(same_version("0.4.0-1", "0.4.0"));
        assert!(!same_version("0.3.3", "0.4.0"));
        assert!(!same_version("", "0.4.0"));
        assert!(!same_version("0.4.01", "0.4.0"));
    }

    #[test]
    fn downloads_are_limited_to_github_hosts_over_https() {
        let p = DownloadPolicy::github();
        let ok = |u: &str| p.allows(&url::Url::parse(u).unwrap());
        for u in [
            "https://github.com/MAlexVR/lumen/releases/download/v1/a.rpm",
            "https://objects.githubusercontent.com/x",
            "https://release-assets.githubusercontent.com/x",
            "https://api.github.com/repos/MAlexVR/lumen/releases/assets/1",
        ] {
            assert!(ok(u), "{u}");
        }
        for u in [
            "http://github.com/x",
            "https://evil.com/x",
            "https://github.com.evil.com/x",
            "https://notgithub.com/x",
            "https://githubusercontent.com/x",
            "ftp://github.com/x",
            "https://user@evil.com/x",
            "https://raw.githubusercontent.com/x",
        ] {
            assert!(!ok(u), "{u}");
        }
    }

    #[test]
    fn progress_is_throttled_but_the_end_always_goes_out() {
        let mut t = Throttle::new(std::time::Duration::from_millis(250));
        let t0 = std::time::Instant::now();
        assert!(t.ready(t0, false), "the first one goes out");
        assert!(!t.ready(t0 + std::time::Duration::from_millis(100), false));
        assert!(t.ready(t0 + std::time::Duration::from_millis(260), false));
        assert!(!t.ready(t0 + std::time::Duration::from_millis(300), false));
        assert!(
            t.ready(t0 + std::time::Duration::from_millis(301), true),
            "the final one is never skipped"
        );
    }

    #[test]
    fn stale_downloads_are_removed_at_startup() {
        let tmp = tempfile::tempdir().unwrap();
        let updates = tmp.path().join("updates");
        std::fs::create_dir_all(updates.join("0.3.5")).unwrap();
        std::fs::create_dir_all(updates.join("0.3.6")).unwrap();
        std::fs::write(updates.join("0.3.5/a.rpm"), "x").unwrap();
        std::fs::write(updates.join("stray.part"), "x").unwrap();
        clean_stale(&updates);
        assert_eq!(std::fs::read_dir(&updates).unwrap().count(), 0);
        clean_stale(&tmp.path().join("missing"));
    }

    // ---- install flow ----

    #[derive(Default)]
    struct FakeSystem {
        runs: Mutex<Vec<Vec<String>>>,
        spawned: Mutex<Vec<Vec<String>>>,
        exits: AtomicUsize,
        commands: Vec<&'static str>,
        /// Installed version answered to the version probe.
        installed: Mutex<String>,
        install_code: Option<i32>,
        install_stderr: String,
        order: Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl System for FakeSystem {
        async fn run(&self, argv: &[String]) -> Result<RunOutput, String> {
            self.runs.lock().unwrap().push(argv.to_vec());
            self.order
                .lock()
                .unwrap()
                .push(format!("run {}", argv.join(" ")));
            if argv
                .first()
                .is_some_and(|a| a == "rpm" || a == "dpkg-query")
            {
                return Ok(RunOutput {
                    code: Some(0),
                    stdout: self.installed.lock().unwrap().clone(),
                    stderr_last: String::new(),
                });
            }
            Ok(RunOutput {
                code: self.install_code.or(Some(0)),
                stdout: String::new(),
                stderr_last: self.install_stderr.clone(),
            })
        }
        fn spawn_detached(&self, argv: &[String]) -> Result<(), String> {
            self.spawned.lock().unwrap().push(argv.to_vec());
            self.order
                .lock()
                .unwrap()
                .push(format!("spawn {}", argv.join(" ")));
            Ok(())
        }
        fn has_command(&self, name: &str) -> bool {
            self.commands.contains(&name)
        }
        fn exit(&self) {
            self.exits.fetch_add(1, Ordering::SeqCst);
            self.order.lock().unwrap().push("exit".into());
        }
    }

    #[derive(Clone, Default)]
    struct Scenario {
        /// Bytes served for the installer.
        file: Vec<u8>,
        sums: Option<String>,
        sig: Option<String>,
        size_override: Option<u64>,
        drop_assets: Vec<String>,
        redirect_to: Option<String>,
        slow_ms: u64,
    }

    fn good(name: &str) -> Scenario {
        let file = payload();
        Scenario {
            sums: Some(format!("{}  {name}\n", sha(&file))),
            sig: Some(SIG.trim().to_string()),
            file,
            ..Default::default()
        }
    }

    fn release(base: &str, name: &str, sc: &Scenario) -> Value {
        let mut assets = vec![
            (
                name.to_string(),
                sc.size_override.unwrap_or(sc.file.len() as u64),
            ),
            ("SHA256SUMS.txt".to_string(), 10),
            (format!("{name}.sig"), 10),
        ];
        assets.retain(|(n, _)| !sc.drop_assets.contains(n));
        json!([{ "tag_name": "v0.4.0", "name": "Lumen v0.4.0", "draft": false, "prerelease": false,
                 "assets": assets.iter().enumerate().map(|(i, (n, size))| json!({
                    "name": n, "size": size, "url": format!("{base}/api/assets/{i}/{n}"),
                    "browser_download_url": format!("{base}/dl/{n}") })).collect::<Vec<_>>() }])
    }

    fn server(
        name: &'static str,
        sc: Scenario,
    ) -> impl Fn(&Recorded, usize) -> Reply + Send + Sync + 'static {
        move |req, _| {
            let base = format!("http://{}", req.header("host").unwrap());
            let p = req.path_and_query.as_str();
            if p.starts_with("/repos/") {
                return Reply::json(&release(&base, name, &sc).to_string());
            }
            if sc.slow_ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(sc.slow_ms));
            }
            if let Some(target) = &sc.redirect_to {
                if p.ends_with(name) {
                    let mut r = Reply::status(302);
                    r = r.with_header("location", target);
                    return r;
                }
            }
            if p.ends_with("SHA256SUMS.txt") {
                return sc.sums.clone().map_or(Reply::status(404), |s| {
                    Reply::bytes(200, "text/plain", s.into_bytes())
                });
            }
            if p.ends_with(".sig") {
                return sc.sig.clone().map_or(Reply::status(404), |s| {
                    Reply::bytes(200, "text/plain", s.into_bytes())
                });
            }
            Reply::bytes(200, "application/octet-stream", sc.file.clone())
        }
    }

    struct Fx {
        srv: TestServer,
        service: UpdateService,
        sys: Arc<FakeSystem>,
        events: Arc<Mutex<Vec<Value>>>,
        cache: std::path::PathBuf,
        tmp: tempfile::TempDir,
    }

    async fn fx(
        kind: InstallKind,
        name: &'static str,
        sc: Scenario,
        sys: FakeSystem,
        current: &str,
    ) -> Fx {
        let srv = TestServer::start(server(name, sc)).await;
        let tmp = tempfile::tempdir().unwrap();
        let core = Arc::new(
            crate::state::AppCore::new(&tmp.path().join("p.db"), tmp.path().join("lib"))
                .await
                .unwrap(),
        );
        let sys = Arc::new(sys);
        let events: Arc<Mutex<Vec<Value>>> = Arc::default();
        let sink = Arc::clone(&events);
        let cache = tmp.path().join("cache/updates");
        let appimage = tmp.path().join("Lumen_0.3.3_amd64.AppImage");
        std::fs::write(&appimage, "old").unwrap();
        let env = UpdateEnv {
            kind,
            arch: "x86_64".into(),
            appimage: Some(appimage),
            updates_dir: cache.clone(),
            system: sys.clone() as Arc<dyn System>,
            emit: Arc::new(move |name, payload| {
                sink.lock()
                    .unwrap()
                    .push(json!({"event": name, "payload": payload}))
            }),
        };
        let service = UpdateService::new(core, tmp.path().join("data"), current, env)
            .with_endpoint(&srv.base(), std::time::Duration::from_secs(10))
            .with_install_options(
                DownloadPolicy::loopback(),
                TEST_PUB.trim(),
                std::time::Duration::ZERO,
            );
        Fx {
            srv,
            service,
            sys,
            events,
            cache,
            tmp,
        }
    }

    fn stages(f: &Fx) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for e in f.events.lock().unwrap().iter() {
            assert_eq!(e["event"], "update://progress");
            let s = e["payload"]["stage"].as_str().unwrap().to_string();
            if out.last() != Some(&s) {
                out.push(s);
            }
        }
        out
    }

    const RPM: &str = "Lumen-0.4.0-1.x86_64.rpm";
    const DEB: &str = "Lumen_0.4.0_amd64.deb";
    const APPIMAGE: &str = "Lumen_0.4.0_amd64.AppImage";
    const EXE: &str = "Lumen_0.4.0_x64-setup.exe";

    fn sys_ok(commands: &[&'static str], installed: &str) -> FakeSystem {
        FakeSystem {
            commands: commands.to_vec(),
            installed: Mutex::new(installed.into()),
            ..Default::default()
        }
    }

    fn code(
        r: &Result<crate::update_install::InstallOutcome, UpdateFailure>,
    ) -> crate::downloads::ErrorCode {
        r.as_ref().unwrap_err().code
    }

    use crate::downloads::ErrorCode;

    #[tokio::test]
    async fn an_rpm_install_downloads_verifies_installs_checks_the_version_and_restarts() {
        let f = fx(
            InstallKind::Rpm,
            RPM,
            good(RPM),
            sys_ok(&["dnf"], "0.4.0"),
            "0.3.3",
        )
        .await;
        let out = f.service.install().await.unwrap();
        assert_eq!(out.version, "0.4.0");
        let file = f.cache.join("0.4.0").join(RPM);
        let runs = f.sys.runs.lock().unwrap().clone();
        assert_eq!(
            runs[0],
            vec![
                "pkexec".to_string(),
                "dnf".into(),
                "install".into(),
                "-y".into(),
                file.to_string_lossy().to_string()
            ]
        );
        assert_eq!(runs[1], vec!["rpm", "-q", "--qf", "%{VERSION}", "lumen"]);
        let order = f.sys.order.lock().unwrap().clone();
        assert_eq!(
            &order[order.len() - 2..],
            ["spawn setsid -f /usr/bin/lumen", "exit"]
        );
        assert_eq!(f.sys.exits.load(Ordering::SeqCst), 1);
        assert_eq!(
            stages(&f),
            ["downloading", "verifying", "installing", "restarting"]
        );
        let last_download = f
            .events
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|e| e["payload"]["stage"] == "downloading")
            .cloned()
            .unwrap();
        assert_eq!(
            last_download["payload"]["downloaded"],
            last_download["payload"]["total"]
        );
        assert!(
            !f.cache.join("0.4.0").exists(),
            "the download is removed once installed"
        );
        assert!(f
            .srv
            .requests()
            .iter()
            .all(|r| r.header("authorization").is_none()));
    }

    #[tokio::test]
    async fn a_deb_install_uses_apt_get() {
        let f = fx(
            InstallKind::Deb,
            DEB,
            good(DEB),
            sys_ok(&["apt-get"], "0.4.0"),
            "0.3.3",
        )
        .await;
        f.service.install().await.unwrap();
        let runs = f.sys.runs.lock().unwrap().clone();
        assert_eq!(&runs[0][..4], ["pkexec", "apt-get", "install", "-y"]);
        assert_eq!(runs[1], vec!["dpkg-query", "-W", "-f=${Version}", "lumen"]);
        assert_eq!(
            f.sys.spawned.lock().unwrap()[0],
            vec!["setsid", "-f", "/usr/bin/lumen"]
        );
    }

    #[tokio::test]
    async fn polkit_denial_is_a_cancellation_and_other_failures_keep_the_last_line() {
        for exit in [126, 127] {
            let mut sys = sys_ok(&["dnf"], "0.3.3");
            sys.install_code = Some(exit);
            let f = fx(InstallKind::Rpm, RPM, good(RPM), sys, "0.3.3").await;
            let r = f.service.install().await;
            assert_eq!(code(&r), ErrorCode::InstallCancelled, "exit {exit}");
            assert_eq!(f.sys.exits.load(Ordering::SeqCst), 0);
            assert!(f.sys.spawned.lock().unwrap().is_empty());
            assert!(!f.cache.join("0.4.0").exists());
        }
        let mut sys = sys_ok(&["dnf"], "0.3.3");
        sys.install_code = Some(1);
        sys.install_stderr = "Error: Transaction failed".into();
        let f = fx(InstallKind::Rpm, RPM, good(RPM), sys, "0.3.3").await;
        let e = f.service.install().await.unwrap_err();
        assert_eq!(e.code, ErrorCode::InstallFailed);
        assert!(e.message.contains("Transaction failed"));
        assert_eq!(f.sys.exits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn an_install_that_leaves_the_old_version_is_a_failure_and_never_restarts() {
        let f = fx(
            InstallKind::Rpm,
            RPM,
            good(RPM),
            sys_ok(&["dnf"], "0.3.3"),
            "0.3.3",
        )
        .await;
        let r = f.service.install().await;
        assert_eq!(code(&r), ErrorCode::InstallFailed);
        assert!(f.sys.spawned.lock().unwrap().is_empty());
        assert_eq!(f.sys.exits.load(Ordering::SeqCst), 0);
        let none = FakeSystem {
            commands: vec![],
            ..Default::default()
        };
        let f = fx(InstallKind::Rpm, RPM, good(RPM), none, "0.3.3").await;
        assert_eq!(
            code(&f.service.install().await),
            ErrorCode::InstallFailed,
            "no package manager found"
        );
    }

    #[tokio::test]
    async fn windows_runs_the_installer_with_the_nsis_update_flags_and_exits() {
        let f = fx(
            InstallKind::WindowsNsis,
            EXE,
            good(EXE),
            sys_ok(&[], ""),
            "0.3.3",
        )
        .await;
        f.service.install().await.unwrap();
        let file = f
            .cache
            .join("0.4.0")
            .join(EXE)
            .to_string_lossy()
            .to_string();
        assert_eq!(
            f.sys.spawned.lock().unwrap().clone(),
            vec![vec![file, "/P".to_string(), "/UPDATE".into(), "/R".into()]]
        );
        assert!(f.sys.runs.lock().unwrap().is_empty());
        assert_eq!(f.sys.exits.load(Ordering::SeqCst), 1);
        assert_eq!(stages(&f).last().map(String::as_str), Some("restarting"));
        assert!(
            f.cache.join("0.4.0").join(EXE).exists(),
            "the installer needs its file after we exit"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_appimage_is_replaced_atomically_and_relaunched() {
        use std::os::unix::fs::PermissionsExt;
        let f = fx(
            InstallKind::Appimage,
            APPIMAGE,
            good(APPIMAGE),
            sys_ok(&[], ""),
            "0.3.3",
        )
        .await;
        let target = f.tmp.path().join("Lumen_0.3.3_amd64.AppImage");
        f.service.install().await.unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), payload());
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert!(!f.tmp.path().join("Lumen_0.3.3_amd64.AppImage.new").exists());
        assert_eq!(
            f.sys.spawned.lock().unwrap()[0],
            vec![
                "setsid".to_string(),
                "-f".into(),
                target.to_string_lossy().to_string()
            ]
        );
        assert_eq!(f.sys.exits.load(Ordering::SeqCst), 1);
        assert!(f.sys.runs.lock().unwrap().is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_unmanaged_install_is_downloaded_verified_and_left_for_the_user() {
        use std::os::unix::fs::PermissionsExt;
        let f = fx(
            InstallKind::Unmanaged,
            APPIMAGE,
            good(APPIMAGE),
            sys_ok(&[], ""),
            "0.3.3",
        )
        .await;
        let e = f.service.install().await.unwrap_err();
        assert_eq!(e.code, ErrorCode::ManualInstallRequired);
        let path = f.cache.join("0.4.0").join(APPIMAGE);
        assert_eq!(e.path.as_deref(), Some(path.to_string_lossy().as_ref()));
        assert_eq!(
            std::fs::read(&path).unwrap(),
            payload(),
            "the verified file stays"
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(f.cache.join("0.4.0"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert!(f.sys.runs.lock().unwrap().is_empty() && f.sys.spawned.lock().unwrap().is_empty());
        assert_eq!(f.sys.exits.load(Ordering::SeqCst), 0);
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["code"], "manual_install_required");
        assert_eq!(v["path"], path.to_string_lossy().as_ref());
    }

    #[tokio::test]
    async fn nothing_is_downloaded_unless_the_release_is_strictly_newer() {
        for current in ["0.4.0", "0.4.1", "1.0.0"] {
            let f = fx(
                InstallKind::Rpm,
                RPM,
                good(RPM),
                sys_ok(&["dnf"], "0.4.0"),
                current,
            )
            .await;
            assert_eq!(
                code(&f.service.install().await),
                ErrorCode::UpToDate,
                "{current}"
            );
            assert_eq!(f.srv.requests().len(), 1, "only the release listing");
            assert!(f.sys.runs.lock().unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn bad_checksums_signatures_and_sizes_stop_everything_and_clean_up() {
        let check = |sc: Scenario, want: ErrorCode| async move {
            let f = fx(
                InstallKind::Rpm,
                RPM,
                sc,
                sys_ok(&["dnf"], "0.4.0"),
                "0.3.3",
            )
            .await;
            let r = f.service.install().await;
            assert_eq!(code(&r), want);
            assert!(f.sys.runs.lock().unwrap().is_empty(), "nothing is executed");
            assert!(f.sys.spawned.lock().unwrap().is_empty());
            assert_eq!(f.sys.exits.load(Ordering::SeqCst), 0);
            assert!(!f.cache.join("0.4.0").exists(), "the folder is cleaned");
        };
        let mut wrong_hash = good(RPM);
        wrong_hash.sums = Some(format!("{}  {RPM}\n", "0".repeat(64)));
        check(wrong_hash, ErrorCode::ChecksumMismatch).await;
        let mut no_entry = good(RPM);
        no_entry.sums = Some(format!("{}  other.rpm\n", sha(&payload())));
        check(no_entry, ErrorCode::ChecksumMismatch).await;
        let mut no_sums = good(RPM);
        no_sums.drop_assets = vec!["SHA256SUMS.txt".into()];
        check(no_sums, ErrorCode::ChecksumMismatch).await;
        let mut no_sig = good(RPM);
        no_sig.drop_assets = vec![format!("{RPM}.sig")];
        check(no_sig, ErrorCode::SignatureMissing).await;
        let mut tampered = good(RPM);
        tampered.file.push(b'!');
        tampered.sums = Some(format!("{}  {RPM}\n", sha(&tampered.file)));
        check(tampered, ErrorCode::SignatureInvalid).await;
        let mut bad_sig = good(RPM);
        bad_sig.sig = Some("bm90IGEgc2lnbmF0dXJl".into());
        check(bad_sig, ErrorCode::SignatureInvalid).await;
        let mut short = good(RPM);
        short.size_override = Some(short.file.len() as u64 + 5);
        check(short, ErrorCode::ChecksumMismatch).await;
    }

    #[tokio::test]
    async fn a_signature_by_another_key_is_rejected() {
        let f = fx(
            InstallKind::Rpm,
            RPM,
            good(RPM),
            sys_ok(&["dnf"], "0.4.0"),
            "0.3.3",
        )
        .await;
        let wrong = UpdateService::new(
            Arc::clone(&f.service.core_for_tests()),
            f.tmp.path().join("data2"),
            "0.3.3",
            f.service.env_for_tests(),
        )
        .with_endpoint(&f.srv.base(), std::time::Duration::from_secs(10))
        .with_install_options(
            DownloadPolicy::loopback(),
            OTHER_PUB.trim(),
            std::time::Duration::ZERO,
        );
        assert_eq!(code(&wrong.install().await), ErrorCode::SignatureInvalid);
        assert!(f.sys.runs.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn missing_installer_assets_and_foreign_hosts_are_refused() {
        let mut gone = good(RPM);
        gone.drop_assets = vec![RPM.into()];
        let f = fx(
            InstallKind::Rpm,
            RPM,
            gone,
            sys_ok(&["dnf"], "0.4.0"),
            "0.3.3",
        )
        .await;
        assert_eq!(code(&f.service.install().await), ErrorCode::NoReleases);
        // A redirect to a host that is not allowed is never followed.
        let mut hop = good(RPM);
        hop.redirect_to = Some(format!("http://127.0.0.1:{}/evil/{RPM}", 1));
        let f = fx(
            InstallKind::Rpm,
            RPM,
            hop,
            sys_ok(&["dnf"], "0.4.0"),
            "0.3.3",
        )
        .await;
        let r = f.service.install().await;
        assert_eq!(code(&r), ErrorCode::InvalidResponse);
        assert!(f.sys.runs.lock().unwrap().is_empty());
        // The asset names must be the exact ones for this install kind.
        let f = fx(
            InstallKind::Deb,
            RPM,
            good(RPM),
            sys_ok(&["apt-get"], "0.4.0"),
            "0.3.3",
        )
        .await;
        assert_eq!(
            code(&f.service.install().await),
            ErrorCode::NoReleases,
            "an rpm is never installed on a deb system"
        );
    }

    #[tokio::test]
    async fn the_token_goes_only_to_the_api_url_and_never_into_errors() {
        let f = fx(
            InstallKind::Rpm,
            RPM,
            good(RPM),
            sys_ok(&["dnf"], "0.4.0"),
            "0.3.3",
        )
        .await;
        f.service.save_token(TOKEN).unwrap();
        f.service.install().await.unwrap();
        let reqs = f.srv.requests();
        assert!(
            reqs.iter()
                .any(|r| r.path_and_query.starts_with("/api/assets/")),
            "the private-repo API URL is used with a token"
        );
        assert!(reqs.iter().all(|r| !r.path_and_query.starts_with("/dl/")));
        for r in reqs
            .iter()
            .filter(|r| r.path_and_query.starts_with("/api/assets/"))
        {
            assert_eq!(
                r.header("authorization").as_deref(),
                Some(&*format!("Bearer {TOKEN}"))
            );
            assert_eq!(
                r.header("accept").as_deref(),
                Some("application/octet-stream")
            );
        }
        let mut bad = good(RPM);
        bad.sums = Some(format!("{}  {RPM}\n", "0".repeat(64)));
        let f = fx(
            InstallKind::Rpm,
            RPM,
            bad,
            sys_ok(&["dnf"], "0.4.0"),
            "0.3.3",
        )
        .await;
        f.service.save_token(TOKEN).unwrap();
        let e = f.service.install().await.unwrap_err();
        assert!(!format!("{e:?} {}", e.message).contains(TOKEN));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cancel_stops_a_download_and_cleans_the_folder() {
        let mut sc = good(RPM);
        sc.file = vec![7u8; 4 * 1024 * 1024];
        let f = fx(
            InstallKind::Rpm,
            RPM,
            sc,
            sys_ok(&["dnf"], "0.4.0"),
            "0.3.3",
        )
        .await;
        let service = Arc::new(f.service);
        let canceller = Arc::clone(&service);
        let events = Arc::clone(&f.events);
        let watcher = tokio::spawn(async move {
            for _ in 0..400 {
                if events.lock().unwrap().iter().any(|e| {
                    e["payload"]["stage"] == "downloading"
                        && e["payload"]["downloaded"].as_u64().unwrap_or(0) > 0
                }) {
                    canceller.cancel();
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            }
        });
        let r = service.install().await;
        watcher.await.unwrap();
        assert_eq!(code(&r), ErrorCode::Cancelled);
        assert!(!f.cache.join("0.4.0").exists());
        assert!(f.sys.runs.lock().unwrap().is_empty());
        assert!(!service.cancel(), "nothing is running any more");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn only_one_install_runs_at_a_time() {
        let mut sc = good(RPM);
        sc.slow_ms = 300;
        let f = fx(
            InstallKind::Rpm,
            RPM,
            sc,
            sys_ok(&["dnf"], "0.4.0"),
            "0.3.3",
        )
        .await;
        let service = Arc::new(f.service);
        let first = {
            let s = Arc::clone(&service);
            tokio::spawn(async move { s.install().await })
        };
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(code(&service.install().await), ErrorCode::AlreadyRunning);
        assert!(first.await.unwrap().is_ok());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_real_system_runs_argv_without_a_shell_and_reports_the_last_stderr_line() {
        let sys = ProcessSystem::new(Arc::new(|| {}));
        let argv = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let out = sys
            .run(&argv(&[
                "/bin/sh",
                "-c",
                "echo out; echo first >&2; echo last >&2; exit 3",
            ]))
            .await
            .unwrap();
        assert_eq!(
            (out.code, out.stdout.as_str(), out.stderr_last.as_str()),
            (Some(3), "out", "last")
        );
        let literal = sys
            .run(&argv(&["/bin/echo", "$(echo injected) ; `id`"]))
            .await
            .unwrap();
        assert_eq!(
            literal.stdout, "$(echo injected) ; `id`",
            "arguments are never interpreted"
        );
        assert!(sys.run(&argv(&["/nonexistent/tool"])).await.is_err());
        assert!(sys.run(&[]).await.is_err());
        assert!(sys.has_command("sh") && !sys.has_command("definitely-not-a-command-xyz"));
    }

    #[test]
    fn the_new_error_codes_serialize_as_snake_case() {
        for (c, s) in [
            (ErrorCode::ChecksumMismatch, "checksum_mismatch"),
            (ErrorCode::SignatureMissing, "signature_missing"),
            (ErrorCode::SignatureInvalid, "signature_invalid"),
            (ErrorCode::InstallCancelled, "install_cancelled"),
            (ErrorCode::InstallFailed, "install_failed"),
            (ErrorCode::ManualInstallRequired, "manual_install_required"),
            (ErrorCode::Cancelled, "cancelled"),
            (ErrorCode::Disk, "disk"),
            (ErrorCode::UpToDate, "up_to_date"),
        ] {
            assert_eq!(serde_json::to_value(c).unwrap(), s);
        }
    }
}
