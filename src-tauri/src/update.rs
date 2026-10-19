//! "Check for updates" (backend only: it never downloads or installs). It asks
//! the GitHub Releases API of [`UPDATE_REPO`] which release is newest, picks
//! the installer for this system and reports it. The repository is private,
//! where GitHub answers 404 without permission, so an optional read-only
//! access token is supported. Pure logic (versions, release selection, asset
//! choice) is separate from the thin HTTP call and fully tested; no tokens
//! ever reach errors, logs or Debug output.

use std::cmp::Ordering;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use lumen_udemy::diagnose::iso8601_utc;
use reqwest::header::{ACCEPT, AUTHORIZATION, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::downloads::{CommandError, ErrorCode};
use crate::session::SessionStore;
use crate::state::AppCore;

pub const UPDATE_REPO: &str = "MAlexVR/lumen";
const API_BASE: &str = "https://api.github.com";
const API_VERSION: &str = "2022-11-28";
const RELEASES_PER_PAGE: u32 = 15;
const TIMEOUT: Duration = Duration::from_secs(10);
const MAX_NOTES_BYTES: usize = 20 * 1024;
const MAX_TOKEN_LEN: usize = 255;
const PRERELEASES_KEY: &str = "updateIncludePrereleases";
const TOKEN_FILE: &str = "update-token";

// ---- versions ----

#[derive(Debug, Clone, PartialEq, Eq)]
enum Ident {
    Num(u64),
    Alpha(String),
}

impl Ord for Ident {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Num(a), Self::Num(b)) => a.cmp(b),
            (Self::Num(_), Self::Alpha(_)) => Ordering::Less,
            (Self::Alpha(_), Self::Num(_)) => Ordering::Greater,
            (Self::Alpha(a), Self::Alpha(b)) => a.cmp(b),
        }
    }
}

impl PartialOrd for Ident {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// `MAJOR.MINOR.PATCH[-pre.release]` with semver precedence; build metadata
/// is ignored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    core: [u64; 3],
    pre: Vec<Ident>,
}

impl Version {
    /// Parses `1.2.3`, `v1.2.3`, `1.2.3-rc.1` and `1.2.3+build`.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.strip_prefix('v').unwrap_or(text);
        let text = text.split('+').next().unwrap_or_default();
        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (text, None),
        };
        let numbers: Vec<u64> = core
            .split('.')
            .map(|n| {
                let plain = !n.is_empty()
                    && n.bytes().all(|b| b.is_ascii_digit())
                    && (n.len() == 1 || !n.starts_with('0'));
                plain.then(|| n.parse().ok()).flatten()
            })
            .collect::<Option<_>>()?;
        let core: [u64; 3] = numbers.try_into().ok()?;
        let pre = match pre {
            None => Vec::new(),
            Some(pre) => pre
                .split('.')
                .map(|id| {
                    let valid = !id.is_empty()
                        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
                    if !valid {
                        None
                    } else if id.bytes().all(|b| b.is_ascii_digit()) {
                        id.parse().ok().map(Ident::Num)
                    } else {
                        Some(Ident::Alpha(id.to_string()))
                    }
                })
                .collect::<Option<_>>()?,
        };
        Some(Self { core, pre })
    }

    pub fn is_prerelease(&self) -> bool {
        !self.pre.is_empty()
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core
            .cmp(&other.core)
            .then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => self.pre.cmp(&other.pre),
            })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.core[0], self.core[1], self.core[2])?;
        if !self.pre.is_empty() {
            let ids: Vec<String> = self
                .pre
                .iter()
                .map(|i| match i {
                    Ident::Num(n) => n.to_string(),
                    Ident::Alpha(a) => a.clone(),
                })
                .collect();
            write!(f, "-{}", ids.join("."))?;
        }
        Ok(())
    }
}

// ---- releases and assets ----

#[derive(Debug, Clone, Deserialize)]
pub struct Asset {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default, rename = "browser_download_url")]
    pub download_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    #[serde(default)]
    pub tag_name: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub assets: Option<Vec<Asset>>,
}

/// The highest-versioned release that is not a draft and has a parsable tag;
/// pre-releases only when `include_prereleases`.
pub fn select(releases: &[Release], include_prereleases: bool) -> Option<&Release> {
    releases
        .iter()
        .filter(|r| !r.draft)
        .filter_map(|r| Some((Version::parse(&r.tag_name)?, r)))
        .filter(|(v, r)| include_prereleases || !(r.prerelease || v.is_prerelease()))
        .max_by(|(a, _), (b, _)| a.cmp(b))
        .map(|(_, r)| r)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AssetKind {
    #[serde(rename = "windows-installer")]
    WindowsInstaller,
    #[serde(rename = "rpm")]
    Rpm,
    #[serde(rename = "deb")]
    Deb,
    #[serde(rename = "appimage")]
    AppImage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Windows,
    Linux,
    Other,
}

/// What the recommended installer is chosen from (injected for tests).
#[derive(Debug, Clone)]
pub struct OsInfo {
    pub family: Family,
    /// `ID` of `/etc/os-release` (lowercase), empty elsewhere.
    pub id: String,
    pub id_like: String,
    /// `std::env::consts::ARCH` style: `x86_64`, `aarch64`.
    pub arch: String,
}

impl OsInfo {
    pub fn from_os_release(text: &str, arch: &str) -> Self {
        let field = |key: &str| {
            text.lines()
                .filter_map(|l| l.split_once('='))
                .find(|(k, _)| k.trim() == key)
                .map(|(_, v)| v.trim().trim_matches(['"', '\'']).to_lowercase())
                .unwrap_or_default()
        };
        Self {
            family: Family::Linux,
            id: field("ID"),
            id_like: field("ID_LIKE"),
            arch: arch.to_string(),
        }
    }

    pub fn detect() -> Self {
        let arch = std::env::consts::ARCH;
        if cfg!(windows) {
            Self {
                family: Family::Windows,
                id: String::new(),
                id_like: String::new(),
                arch: arch.into(),
            }
        } else if cfg!(target_os = "linux") {
            let text = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
            Self::from_os_release(&text, arch)
        } else {
            Self {
                family: Family::Other,
                id: String::new(),
                id_like: String::new(),
                arch: arch.into(),
            }
        }
    }

    fn linux_kind(&self) -> AssetKind {
        let families = format!("{} {}", self.id, self.id_like);
        let has = |names: &[&str]| names.iter().any(|n| families.contains(n));
        if has(&["fedora", "rhel", "centos", "suse"]) {
            AssetKind::Rpm
        } else if has(&["debian", "ubuntu"]) {
            AssetKind::Deb
        } else {
            AssetKind::AppImage
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetInfo {
    pub name: String,
    pub size: u64,
    pub download_url: String,
    pub kind: AssetKind,
}

const ARCHES: [(&str, &[&str]); 2] = [
    ("x86_64", &["x86_64", "amd64", "x64"]),
    ("aarch64", &["aarch64", "arm64"]),
];

/// 0 = built for this CPU, 1 = names no CPU, 2 = built for another CPU.
fn arch_rank(name: &str, arch: &str) -> u8 {
    let lower = name.to_lowercase();
    let mine = ARCHES
        .iter()
        .find(|(a, _)| *a == arch)
        .map_or(&[][..], |(_, al)| al);
    if mine.iter().any(|a| lower.contains(a)) {
        0
    } else if ARCHES
        .iter()
        .flat_map(|(_, al)| *al)
        .any(|a| lower.contains(a))
    {
        2
    } else {
        1
    }
}

/// The installer for this system, or `None` when the release has none.
pub fn pick_asset(assets: &[Asset], os: &OsInfo) -> Option<AssetInfo> {
    let kind = match os.family {
        Family::Windows => AssetKind::WindowsInstaller,
        Family::Linux => os.linux_kind(),
        Family::Other => return None,
    };
    let matches = |name: &str| {
        let lower = name.to_lowercase();
        match kind {
            AssetKind::WindowsInstaller => lower.ends_with("-setup.exe"),
            AssetKind::Rpm => lower.ends_with(".rpm"),
            AssetKind::Deb => lower.ends_with(".deb"),
            AssetKind::AppImage => lower.ends_with(".appimage"),
        }
    };
    assets
        .iter()
        .filter(|a| matches(&a.name))
        .map(|a| (arch_rank(&a.name, &os.arch), a))
        .filter(|(rank, _)| *rank < 2)
        .min_by_key(|(rank, _)| *rank)
        .map(|(_, a)| AssetInfo {
            name: a.name.clone(),
            size: a.size,
            download_url: a.download_url.clone(),
            kind,
        })
}

// ---- service ----

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current: String,
    pub latest: Option<String>,
    pub is_newer: bool,
    pub is_prerelease: bool,
    pub name: String,
    pub published_at: Option<String>,
    /// Markdown release notes, capped at about 20 KB.
    pub notes: String,
    pub html_url: Option<String>,
    pub asset: Option<AssetInfo>,
    pub checked_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSettings {
    pub include_prereleases: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TokenStatus {
    pub saved: bool,
}

pub struct UpdateService {
    core: Arc<AppCore>,
    tokens: SessionStore,
    current: String,
    os: OsInfo,
    base: String,
    timeout: Duration,
}

impl fmt::Debug for UpdateService {
    // Deliberately omits the token store.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UpdateService")
            .field("current", &self.current)
            .finish_non_exhaustive()
    }
}

fn err(code: ErrorCode, message: impl Into<String>) -> CommandError {
    CommandError::new(code, message)
}

/// Cuts `text` to `max` bytes on a character boundary, marking the cut.
fn cap_notes(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max - '…'.len_utf8();
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

impl UpdateService {
    pub fn new(core: Arc<AppCore>, data_dir: PathBuf, current: &str, os: OsInfo) -> Self {
        Self {
            core,
            tokens: SessionStore::new(data_dir.join(TOKEN_FILE)),
            current: current.to_string(),
            os,
            base: API_BASE.to_string(),
            timeout: TIMEOUT,
        }
    }

    /// Points the service at another server and timeout (tests).
    #[cfg(test)]
    #[must_use]
    pub fn with_endpoint(mut self, base: &str, timeout: Duration) -> Self {
        self.base = base.trim_end_matches('/').to_string();
        self.timeout = timeout;
        self
    }

    pub fn settings(&self) -> Result<UpdateSettings, CommandError> {
        let include = self
            .core
            .setting(PRERELEASES_KEY)
            .map_err(|e| err(ErrorCode::Internal, e.to_string()))?
            .is_none_or(|v| v != "false");
        Ok(UpdateSettings {
            include_prereleases: include,
        })
    }

    pub fn set_include_prereleases(&self, value: bool) -> Result<(), CommandError> {
        self.core
            .set_setting(PRERELEASES_KEY, if value { "true" } else { "false" })
            .map_err(|e| err(ErrorCode::Internal, e.to_string()))
    }

    pub fn token_status(&self) -> Result<TokenStatus, CommandError> {
        Ok(TokenStatus {
            saved: self.load_token()?.is_some(),
        })
    }

    fn load_token(&self) -> Result<Option<String>, CommandError> {
        self.tokens.load().map_err(|e| {
            err(
                ErrorCode::Internal,
                format!("cannot read the saved token: {e}"),
            )
        })
    }

    /// Stores a read-only access token privately (never echoed back).
    pub fn save_token(&self, token: &str) -> Result<(), CommandError> {
        let token = token.trim();
        if token.is_empty() {
            return Err(err(ErrorCode::InvalidToken, "the token is empty"));
        }
        if token.len() > MAX_TOKEN_LEN {
            return Err(err(ErrorCode::InvalidToken, "the token is too long"));
        }
        if !token.chars().all(|c| c.is_ascii_graphic()) {
            return Err(err(
                ErrorCode::InvalidToken,
                "the token contains spaces or invalid characters",
            ));
        }
        self.tokens
            .save(token)
            .map_err(|e| err(ErrorCode::Internal, format!("cannot store the token: {e}")))
    }

    pub fn clear_token(&self) -> Result<(), CommandError> {
        self.tokens
            .clear()
            .map_err(|e| err(ErrorCode::Internal, format!("cannot remove the token: {e}")))
    }

    fn unauthorized(has_token: bool) -> CommandError {
        err(
            ErrorCode::UpdateUnauthorized,
            if has_token {
                "the saved token was rejected or lacks access to the repository"
            } else {
                "the repository is private: save a read-only access token"
            },
        )
    }

    async fn fetch_listing(&self, token: Option<&str>) -> Result<Vec<Release>, CommandError> {
        let url = format!(
            "{}/repos/{UPDATE_REPO}/releases?per_page={RELEASES_PER_PAGE}",
            self.base
        );
        let client = reqwest::Client::builder()
            .timeout(self.timeout)
            .build()
            .map_err(|e| err(ErrorCode::Internal, e.without_url().to_string()))?;
        let mut req = client
            .get(url)
            .header(ACCEPT, "application/vnd.github+json")
            .header("x-github-api-version", API_VERSION)
            .header(USER_AGENT, format!("lumen/{}", self.current));
        if let Some(token) = token {
            let mut value = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|_| err(ErrorCode::InvalidToken, "the saved token cannot be sent"))?;
            value.set_sensitive(true);
            req = req.header(AUTHORIZATION, value);
        }
        let resp = req.send().await.map_err(|e| {
            let what = if e.is_timeout() {
                "the request timed out".to_string()
            } else {
                e.without_url().to_string()
            };
            err(ErrorCode::Network, format!("cannot reach GitHub: {what}"))
        })?;
        let status = resp.status().as_u16();
        if status != 200 {
            let header = |name: &str| {
                resp.headers()
                    .get(name)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string)
            };
            let exhausted = header("x-ratelimit-remaining").as_deref() == Some("0");
            return Err(match status {
                429 | 403 if status == 429 || exhausted => {
                    let reset = header("x-ratelimit-reset")
                        .and_then(|r| r.parse::<u64>().ok())
                        .map(|r| format!(" Try again after {}.", iso8601_utc(r)))
                        .unwrap_or_default();
                    err(
                        ErrorCode::RateLimited,
                        format!("GitHub's rate limit was reached.{reset}"),
                    )
                }
                401 | 403 | 404 => Self::unauthorized(token.is_some()),
                500.. => err(ErrorCode::Network, format!("GitHub answered HTTP {status}")),
                _ => err(
                    ErrorCode::InvalidResponse,
                    format!("GitHub answered HTTP {status}"),
                ),
            });
        }
        let text = resp.text().await.map_err(|e| {
            err(
                ErrorCode::Network,
                format!("cannot read GitHub's answer: {}", e.without_url()),
            )
        })?;
        let value: Value = serde_json::from_str(&text).map_err(|_| {
            err(
                ErrorCode::InvalidResponse,
                "GitHub's answer is not valid JSON",
            )
        })?;
        let list = value.as_array().ok_or_else(|| {
            err(
                ErrorCode::InvalidResponse,
                "GitHub's answer is not a release list",
            )
        })?;
        Ok(list
            .iter()
            .filter_map(|r| serde_json::from_value(r.clone()).ok())
            .collect())
    }

    /// Asks GitHub for the newest release and reports it with the installer
    /// for this system. Nothing is downloaded.
    pub async fn check(&self) -> Result<UpdateInfo, CommandError> {
        let current = Version::parse(&self.current).ok_or_else(|| {
            err(
                ErrorCode::Internal,
                "the app version is not a valid version",
            )
        })?;
        let include = self.settings()?.include_prereleases;
        let token = self.load_token()?;
        let releases = self.fetch_listing(token.as_deref()).await?;
        let release = select(&releases, include).ok_or_else(|| {
            err(
                ErrorCode::NoReleases,
                "the repository has no usable release",
            )
        })?;
        let latest = Version::parse(&release.tag_name).ok_or_else(|| {
            err(
                ErrorCode::InvalidResponse,
                "the release tag is not a version",
            )
        })?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        Ok(UpdateInfo {
            current: current.to_string(),
            is_newer: latest > current,
            is_prerelease: release.prerelease || latest.is_prerelease(),
            latest: Some(latest.to_string()),
            name: release
                .name
                .clone()
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| release.tag_name.clone()),
            published_at: release.published_at.clone(),
            notes: cap_notes(release.body.as_deref().unwrap_or_default(), MAX_NOTES_BYTES),
            html_url: release.html_url.clone(),
            asset: pick_asset(release.assets.as_deref().unwrap_or_default(), &self.os),
            checked_at: iso8601_utc(now),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_download::testsrv::{Recorded, Reply, TestServer};
    use serde_json::{json, Value};
    use std::time::Duration;

    const TOKEN: &str = "ghp_SECRETTOKEN1234567890";

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap_or_else(|| panic!("{s}"))
    }

    #[test]
    fn versions_parse_with_an_optional_v_and_pre_release() {
        assert_eq!(v("v0.3.1").to_string(), "0.3.1");
        assert_eq!(v("1.2.3-rc.1").to_string(), "1.2.3-rc.1");
        assert_eq!(v("1.2.3+build5").to_string(), "1.2.3");
        for bad in [
            "",
            "v",
            "1.2",
            "1.2.3.4",
            "x.y.z",
            "1.2.-3",
            "1.2.3-",
            "v1.2.3-rc..1",
            "01.2.3x",
            "1.2.3 ",
        ] {
            assert!(Version::parse(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn versions_compare_by_semver_precedence() {
        let ordered = [
            "0.3.9",
            "0.3.10",
            "0.4.0-alpha",
            "0.4.0-alpha.1",
            "0.4.0-alpha.beta",
            "0.4.0-beta",
            "0.4.0-beta.2",
            "0.4.0-beta.11",
            "0.4.0-rc.1",
            "0.4.0",
            "0.10.0",
            "1.0.0",
        ];
        for pair in ordered.windows(2) {
            assert!(v(pair[0]) < v(pair[1]), "{} < {}", pair[0], pair[1]);
        }
        assert_eq!(v("1.0.0+a"), v("1.0.0+b"));
        assert!(
            v("1.0.0-1") < v("1.0.0-a"),
            "numeric identifiers sort before alphanumeric"
        );
        assert!(v("1.0.0-2") < v("1.0.0-10"));
        assert!(v("v2.0.0") > v("1.99.99"));
    }

    fn rel(tag: &str, draft: bool, pre: bool) -> Release {
        serde_json::from_value(
            json!({"tag_name": tag, "draft": draft, "prerelease": pre, "name": tag}),
        )
        .unwrap()
    }

    #[test]
    fn the_highest_valid_release_wins_and_drafts_and_odd_tags_are_ignored() {
        let rs = vec![
            rel("v0.3.9", false, false),
            rel("v0.3.10", false, true),
            rel("v9.9.9", true, false),
            rel("nightly", false, false),
            rel("v0.4.0-rc.1", false, true),
            rel("release-1", false, false),
        ];
        assert_eq!(select(&rs, true).unwrap().tag_name, "v0.4.0-rc.1");
        assert_eq!(
            select(&rs, false).unwrap().tag_name,
            "v0.3.9",
            "pre-releases are excluded"
        );
        let only_pre = vec![rel("v0.1.0", false, true)];
        assert!(select(&only_pre, false).is_none());
        assert!(select(&[], true).is_none());
        assert!(select(
            &[rel("v1.0.0", true, false), rel("junk", false, false)],
            true
        )
        .is_none());
        let flag_missing = vec![rel("v1.0.0-beta.1", false, false)];
        assert!(
            select(&flag_missing, false).is_none(),
            "a pre-release tag counts even when GitHub did not flag it"
        );
    }

    fn asset(name: &str) -> Asset {
        serde_json::from_value(json!({"name": name, "size": 10, "browser_download_url": format!("https://dl.test/{name}")})).unwrap()
    }

    fn assets() -> Vec<Asset> {
        [
            "Lumen_0.4.0_x64-setup.exe",
            "Lumen-0.4.0-1.x86_64.rpm",
            "Lumen-0.4.0-1.aarch64.rpm",
            "lumen_0.4.0_amd64.deb",
            "Lumen_0.4.0_amd64.AppImage",
            "notes.txt",
        ]
        .iter()
        .map(|n| asset(n))
        .collect()
    }

    fn os(family: Family, id: &str, like: &str) -> OsInfo {
        OsInfo {
            family,
            id: id.into(),
            id_like: like.into(),
            arch: "x86_64".into(),
        }
    }

    #[test]
    fn the_recommended_asset_depends_on_the_system() {
        let pick = |o: &OsInfo, a: &[Asset]| pick_asset(a, o).map(|x| (x.kind, x.name));
        assert_eq!(
            pick(&os(Family::Windows, "", ""), &assets()),
            Some((
                AssetKind::WindowsInstaller,
                "Lumen_0.4.0_x64-setup.exe".into()
            ))
        );
        assert_eq!(
            pick(&os(Family::Linux, "fedora", ""), &assets()),
            Some((AssetKind::Rpm, "Lumen-0.4.0-1.x86_64.rpm".into()))
        );
        assert_eq!(
            pick(&os(Family::Linux, "nobara", "fedora"), &assets())
                .unwrap()
                .0,
            AssetKind::Rpm
        );
        assert_eq!(
            pick(
                &os(Family::Linux, "opensuse-tumbleweed", "opensuse suse"),
                &assets()
            )
            .unwrap()
            .0,
            AssetKind::Rpm
        );
        assert_eq!(
            pick(&os(Family::Linux, "rocky", "rhel centos fedora"), &assets())
                .unwrap()
                .0,
            AssetKind::Rpm
        );
        assert_eq!(
            pick(&os(Family::Linux, "ubuntu", "debian"), &assets()),
            Some((AssetKind::Deb, "lumen_0.4.0_amd64.deb".into()))
        );
        assert_eq!(
            pick(&os(Family::Linux, "linuxmint", "ubuntu debian"), &assets())
                .unwrap()
                .0,
            AssetKind::Deb
        );
        assert_eq!(
            pick(&os(Family::Linux, "arch", ""), &assets()),
            Some((AssetKind::AppImage, "Lumen_0.4.0_amd64.AppImage".into()))
        );
        assert_eq!(
            pick(&os(Family::Linux, "", ""), &assets()).unwrap().0,
            AssetKind::AppImage
        );
        assert_eq!(pick(&os(Family::Other, "macos", ""), &assets()), None);
        let no_rpm: Vec<Asset> = assets()
            .into_iter()
            .filter(|a| !a.name.ends_with(".rpm"))
            .collect();
        assert_eq!(
            pick(&os(Family::Linux, "fedora", ""), &no_rpm),
            None,
            "no fallback to another kind"
        );
        assert_eq!(pick(&os(Family::Windows, "", ""), &[]), None);
        let upper = vec![asset("LUMEN-SETUP.EXE")];
        assert_eq!(
            pick(&os(Family::Windows, "", ""), &upper).unwrap().0,
            AssetKind::WindowsInstaller
        );
        let mut arm = os(Family::Linux, "fedora", "");
        arm.arch = "aarch64".into();
        assert_eq!(
            pick(&arm, &assets()).unwrap().1,
            "Lumen-0.4.0-1.aarch64.rpm"
        );
    }

    #[test]
    fn os_release_is_parsed_with_quotes_and_comments() {
        let o = OsInfo::from_os_release(
            "# c\nNAME=\"Nobara Linux\"\nID=nobara\nID_LIKE=\"rhel centos fedora\"\n\nBAD LINE\n",
            "x86_64",
        );
        assert_eq!(
            (o.family, o.id.as_str(), o.id_like.as_str()),
            (Family::Linux, "nobara", "rhel centos fedora")
        );
        assert_eq!(OsInfo::from_os_release("", "x86_64").id, "");
    }

    // ---- service ----

    struct Fixture {
        srv: TestServer,
        service: UpdateService,
        _tmp: tempfile::TempDir,
        dir: std::path::PathBuf,
        core: Arc<AppCore>,
    }

    fn release_json(tag: &str, pre: bool, body: &str) -> Value {
        json!({"tag_name": tag, "name": format!("Lumen {tag}"), "draft": false, "prerelease": pre,
               "published_at": "2026-10-01T12:00:00Z", "html_url": format!("https://github.com/MAlexVR/lumen/releases/tag/{tag}"),
               "body": body, "assets": assets().iter().map(|a| json!({"name": a.name, "size": a.size, "browser_download_url": a.download_url})).collect::<Vec<_>>()})
    }

    fn listing() -> String {
        json!([
            release_json("v0.3.9", false, "old"), release_json("v0.4.0-rc.1", true, "rc notes"),
            release_json("v0.3.10", true, "## Notes\nhello"), {"tag_name": "junk", "draft": false, "assets": []},
            {"tag_name": "v9.0.0", "draft": true, "prerelease": false}
        ]).to_string()
    }

    async fn fixture(
        handler: impl Fn(&Recorded, usize) -> Reply + Send + Sync + 'static,
        current: &str,
        os: OsInfo,
    ) -> Fixture {
        let srv = TestServer::start(handler).await;
        let tmp = tempfile::tempdir().unwrap();
        let core = Arc::new(
            AppCore::new(&tmp.path().join("p.db"), tmp.path().join("lib"))
                .await
                .unwrap(),
        );
        let dir = tmp.path().join("data");
        let service = UpdateService::new(Arc::clone(&core), dir.clone(), current, os)
            .with_endpoint(&srv.base(), Duration::from_secs(5));
        Fixture {
            srv,
            service,
            _tmp: tmp,
            dir,
            core,
        }
    }

    fn fedora() -> OsInfo {
        os(Family::Linux, "fedora", "")
    }

    #[tokio::test]
    async fn a_check_picks_the_newest_release_with_notes_and_the_asset_for_this_system() {
        let f = fixture(|_, _| Reply::json(&listing()), "0.3.1", fedora()).await;
        let info = f.service.check().await.unwrap();
        let j = serde_json::to_value(&info).unwrap();
        assert_eq!(j["current"], "0.3.1");
        assert_eq!(j["latest"], "0.4.0-rc.1");
        assert_eq!(
            (j["isNewer"].clone(), j["isPrerelease"].clone()),
            (json!(true), json!(true))
        );
        assert_eq!(j["name"], "Lumen v0.4.0-rc.1");
        assert_eq!(j["publishedAt"], "2026-10-01T12:00:00Z");
        assert_eq!(j["notes"], "rc notes");
        assert_eq!(
            j["htmlUrl"],
            "https://github.com/MAlexVR/lumen/releases/tag/v0.4.0-rc.1"
        );
        assert_eq!(j["asset"]["kind"], "rpm");
        assert_eq!(j["asset"]["name"], "Lumen-0.4.0-1.x86_64.rpm");
        assert_eq!(j["asset"]["size"], 10);
        assert_eq!(
            j["asset"]["downloadUrl"],
            "https://dl.test/Lumen-0.4.0-1.x86_64.rpm"
        );
        assert!(j["checkedAt"].as_str().unwrap().ends_with('Z'));
        let req = &f.srv.requests()[0];
        assert!(
            req.path_and_query
                .starts_with("/repos/MAlexVR/lumen/releases?per_page=15"),
            "{}",
            req.path_and_query
        );
    }

    #[tokio::test]
    async fn the_request_headers_and_the_token_rule() {
        let f = fixture(|_, _| Reply::json(&listing()), "0.3.1", fedora()).await;
        f.service.check().await.unwrap();
        let req = &f.srv.requests()[0];
        assert_eq!(
            req.header("accept").as_deref(),
            Some("application/vnd.github+json")
        );
        assert_eq!(
            req.header("x-github-api-version").as_deref(),
            Some("2022-11-28")
        );
        assert_eq!(req.header("user-agent").as_deref(), Some("lumen/0.3.1"));
        assert!(
            req.header("authorization").is_none(),
            "no token, no Authorization header"
        );
        f.service.save_token(&format!("  {TOKEN}\n")).unwrap();
        f.service.check().await.unwrap();
        let req = &f.srv.requests()[1];
        assert_eq!(
            req.header("authorization").as_deref(),
            Some(&*format!("Bearer {TOKEN}"))
        );
        f.service.clear_token().unwrap();
        f.service.check().await.unwrap();
        assert!(f.srv.requests()[2].header("authorization").is_none());
    }

    #[tokio::test]
    async fn pre_releases_follow_the_persisted_setting_and_default_to_included() {
        let f = fixture(|_, _| Reply::json(&listing()), "0.3.1", fedora()).await;
        assert!(f.service.settings().unwrap().include_prereleases);
        assert_eq!(
            f.service.check().await.unwrap().latest.as_deref(),
            Some("0.4.0-rc.1")
        );
        f.service.set_include_prereleases(false).unwrap();
        assert!(!f.service.settings().unwrap().include_prereleases);
        let info = f.service.check().await.unwrap();
        assert_eq!(
            (info.latest.as_deref(), info.is_prerelease),
            (Some("0.3.9"), false)
        );
        assert!(info.is_newer);
        drop(f.service);
        let again = UpdateService::new(Arc::clone(&f.core), f.dir.clone(), "0.3.1", fedora());
        assert!(
            !again.settings().unwrap().include_prereleases,
            "persisted in the settings table"
        );
        assert_eq!(
            serde_json::to_value(again.settings().unwrap()).unwrap(),
            json!({"includePrereleases": false})
        );
    }

    #[tokio::test]
    async fn newer_is_false_when_current_or_ahead_and_the_asset_follows_the_os() {
        let one = json!([release_json("v0.3.1", false, "x")]).to_string();
        let f = fixture(
            move |_, _| Reply::json(&one),
            "0.3.1",
            os(Family::Windows, "", ""),
        )
        .await;
        let info = f.service.check().await.unwrap();
        assert!(!info.is_newer);
        assert_eq!(info.asset.unwrap().kind, AssetKind::WindowsInstaller);
        let two = json!([release_json("v0.3.1", false, "x")]).to_string();
        let f = fixture(
            move |_, _| Reply::json(&two),
            "0.3.10",
            os(Family::Linux, "ubuntu", "debian"),
        )
        .await;
        let info = f.service.check().await.unwrap();
        assert!(!info.is_newer && info.latest.as_deref() == Some("0.3.1"));
        assert_eq!(
            serde_json::to_value(info.asset.unwrap().kind).unwrap(),
            "deb"
        );
        let pre_current = json!([release_json("v0.4.0", false, "x")]).to_string();
        let f = fixture(
            move |_, _| Reply::json(&pre_current),
            "0.4.0-rc.1",
            fedora(),
        )
        .await;
        assert!(
            f.service.check().await.unwrap().is_newer,
            "the final release is newer than its rc"
        );
    }

    #[tokio::test]
    async fn a_missing_asset_and_long_notes_are_handled() {
        let body = "é".repeat(30_000);
        let mut r = release_json("v1.0.0", false, &body);
        r["assets"] = json!([]);
        let list = json!([r]).to_string();
        let f = fixture(move |_, _| Reply::json(&list), "0.1.0", fedora()).await;
        let info = f.service.check().await.unwrap();
        assert!(info.asset.is_none());
        assert!(info.notes.len() <= 20 * 1024 && info.notes.len() > 15_000);
        assert!(info.notes.chars().all(|c| c == 'é' || c == '…'));
        let null_body =
            json!([{"tag_name": "v1.0.0", "draft": false, "body": null, "assets": null}])
                .to_string();
        let f = fixture(move |_, _| Reply::json(&null_body), "0.1.0", fedora()).await;
        let info = f.service.check().await.unwrap();
        assert_eq!((info.notes.as_str(), info.name.as_str()), ("", "v1.0.0"));
    }

    #[tokio::test]
    async fn no_valid_release_is_its_own_error() {
        for body in [
            "[]",
            r#"[{"tag_name":"junk"},{"tag_name":"v1.0.0","draft":true}]"#,
        ] {
            let f = fixture(move |_, _| Reply::json(body), "0.1.0", fedora()).await;
            assert_eq!(
                f.service.check().await.unwrap_err().code,
                ErrorCode::NoReleases
            );
        }
    }

    fn status_reply(status: u16, headers: &[(&str, &str)]) -> Reply {
        let mut r = Reply::status(status);
        for (k, val) in headers {
            r = r.with_header(k, val);
        }
        r
    }

    #[tokio::test]
    async fn private_repository_and_rejected_tokens_have_distinct_explanations() {
        for status in [404u16, 401, 403] {
            let f = fixture(move |_, _| status_reply(status, &[]), "0.1.0", fedora()).await;
            let e = f.service.check().await.unwrap_err();
            assert_eq!(e.code, ErrorCode::UpdateUnauthorized, "{status}");
            assert!(
                e.message.contains("private") && e.message.contains("read-only access token"),
                "{}",
                e.message
            );
            f.service.save_token(TOKEN).unwrap();
            let e = f.service.check().await.unwrap_err();
            assert_eq!(e.code, ErrorCode::UpdateUnauthorized);
            assert!(
                e.message
                    .contains("saved token was rejected or lacks access"),
                "{}",
                e.message
            );
            assert!(!e.message.contains(TOKEN));
        }
    }

    #[tokio::test]
    async fn rate_limits_are_recognised_by_their_headers() {
        let f = fixture(
            |_, _| {
                status_reply(
                    403,
                    &[
                        ("x-ratelimit-remaining", "0"),
                        ("x-ratelimit-reset", "1790000000"),
                    ],
                )
            },
            "0.1.0",
            fedora(),
        )
        .await;
        let e = f.service.check().await.unwrap_err();
        assert_eq!(e.code, ErrorCode::RateLimited);
        assert!(e.message.contains("2026-09-21T"), "{}", e.message);
        let f = fixture(
            |_, _| status_reply(429, &[("x-ratelimit-remaining", "0")]),
            "0.1.0",
            fedora(),
        )
        .await;
        let e = f.service.check().await.unwrap_err();
        assert_eq!(e.code, ErrorCode::RateLimited);
        let f = fixture(|_, _| status_reply(429, &[]), "0.1.0", fedora()).await;
        assert_eq!(
            f.service.check().await.unwrap_err().code,
            ErrorCode::RateLimited,
            "429 is always a rate limit"
        );
        let f = fixture(
            |_, _| status_reply(403, &[("x-ratelimit-remaining", "42")]),
            "0.1.0",
            fedora(),
        )
        .await;
        assert_eq!(
            f.service.check().await.unwrap_err().code,
            ErrorCode::UpdateUnauthorized
        );
    }

    #[tokio::test]
    async fn broken_answers_servers_down_and_timeouts_have_stable_codes() {
        let f = fixture(|_, _| Reply::json("<<not json>>"), "0.1.0", fedora()).await;
        assert_eq!(
            f.service.check().await.unwrap_err().code,
            ErrorCode::InvalidResponse
        );
        let f = fixture(
            |_, _| Reply::json(r#"{"message":"object"}"#),
            "0.1.0",
            fedora(),
        )
        .await;
        assert_eq!(
            f.service.check().await.unwrap_err().code,
            ErrorCode::InvalidResponse
        );
        let f = fixture(|_, _| Reply::status(502), "0.1.0", fedora()).await;
        let e = f.service.check().await.unwrap_err();
        assert_eq!(e.code, ErrorCode::Network);
        assert!(e.message.contains("502"));
        let f = fixture(|_, _| Reply::status(418), "0.1.0", fedora()).await;
        assert_eq!(
            f.service.check().await.unwrap_err().code,
            ErrorCode::InvalidResponse
        );
        let dead = UpdateService::new(Arc::clone(&f.core), f.dir.clone(), "0.1.0", fedora())
            .with_endpoint("http://127.0.0.1:1", Duration::from_secs(1));
        assert_eq!(dead.check().await.unwrap_err().code, ErrorCode::Network);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_slow_server_times_out_as_a_network_error() {
        let f = fixture(
            |_, _| {
                std::thread::sleep(Duration::from_millis(600));
                Reply::json("[]")
            },
            "0.1.0",
            fedora(),
        )
        .await;
        let slow = UpdateService::new(Arc::clone(&f.core), f.dir.clone(), "0.1.0", fedora())
            .with_endpoint(&f.srv.base(), Duration::from_millis(150));
        let e = slow.check().await.unwrap_err();
        assert_eq!(e.code, ErrorCode::Network);
        assert!(
            e.message.contains("timed out") || e.message.contains("timeout"),
            "{}",
            e.message
        );
    }

    #[tokio::test]
    async fn the_token_is_stored_privately_validated_and_never_leaks() {
        let f = fixture(|_, _| status_reply(401, &[]), "0.1.0", fedora()).await;
        assert!(!f.service.token_status().unwrap().saved);
        f.service.save_token(TOKEN).unwrap();
        assert!(f.service.token_status().unwrap().saved);
        let file = f.dir.join("update-token");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), TOKEN);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert_eq!(
            serde_json::to_value(f.service.token_status().unwrap()).unwrap(),
            json!({"saved": true})
        );
        for bad in [
            "",
            "   ",
            "\n",
            &"a".repeat(256),
            "has space",
            "tab\there",
            "new\nline",
            "ctl\u{1}x",
        ] {
            let e = f.service.save_token(bad).unwrap_err();
            assert_eq!(e.code, ErrorCode::InvalidToken, "{bad:?}");
            assert!(!e.message.contains(TOKEN));
        }
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            TOKEN,
            "a rejected token leaves the saved one alone"
        );
        assert!(f.service.save_token(&"a".repeat(255)).is_ok());
        f.service.save_token(TOKEN).unwrap();
        let shown = format!("{:?}", f.service);
        assert!(!shown.contains(TOKEN), "{shown}");
        let e = f.service.check().await.unwrap_err();
        assert!(!format!("{e:?} {}", e.message).contains(TOKEN));
        let dead = UpdateService::new(Arc::clone(&f.core), f.dir.clone(), "0.1.0", fedora())
            .with_endpoint("http://127.0.0.1:1", Duration::from_secs(1));
        let e = dead.check().await.unwrap_err();
        assert!(!format!("{e:?} {}", e.message).contains(TOKEN));
        f.service.clear_token().unwrap();
        f.service.clear_token().unwrap();
        assert!(!f.service.token_status().unwrap().saved);
        assert!(!file.exists());
    }

    #[test]
    fn the_new_error_codes_serialize_as_snake_case() {
        for (c, s) in [
            (ErrorCode::UpdateUnauthorized, "update_unauthorized"),
            (ErrorCode::RateLimited, "rate_limited"),
            (ErrorCode::NoReleases, "no_releases"),
            (ErrorCode::InvalidResponse, "invalid_response"),
        ] {
            assert_eq!(serde_json::to_value(c).unwrap(), s);
        }
        assert_eq!(UPDATE_REPO, "MAlexVR/lumen");
    }
}
