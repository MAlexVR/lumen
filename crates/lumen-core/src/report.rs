//! Download report and progress types shared by every course platform.

use serde::{Deserialize, Serialize};

/// What went wrong. Serialized as snake_case so the UI can switch on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueKind {
    Video,
    VideoNotResolved,
    Resource,
    ModulePage,
    EmptyLesson,
    Course,
}

/// Errors block the `.complete` marker; warnings are informational.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Warning,
    Error,
}

/// One failed or unresolved item. `url` never carries a query string or
/// credentials (see [`redact_url`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub lesson: Option<String>,
    pub kind: IssueKind,
    pub severity: Severity,
    pub host: Option<String>,
    pub url: Option<String>,
    pub message: String,
}

impl Issue {
    pub fn new(
        kind: IssueKind,
        severity: Severity,
        lesson: Option<&str>,
        url: Option<&str>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            lesson: lesson.map(str::to_string),
            kind,
            severity,
            host: url.and_then(url_host),
            url: url.map(redact_url),
            message: message.into(),
        }
    }

    pub fn error(
        kind: IssueKind,
        lesson: Option<&str>,
        url: Option<&str>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(kind, Severity::Error, lesson, url, message)
    }

    pub fn warning(
        kind: IssueKind,
        lesson: Option<&str>,
        url: Option<&str>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(kind, Severity::Warning, lesson, url, message)
    }
}

/// Final outcome of downloading one group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadReport {
    pub platform: String,
    pub group: String,
    pub cancelled: bool,
    pub total_lessons: u32,
    pub completed_lessons: u32,
    pub complete: bool,
    pub issues: Vec<Issue>,
}

impl DownloadReport {
    pub fn new(platform: &str, group: &str) -> Self {
        Self {
            platform: platform.to_string(),
            group: group.to_string(),
            cancelled: false,
            total_lessons: 0,
            completed_lessons: 0,
            complete: false,
            issues: Vec::new(),
        }
    }

    pub fn push(&mut self, issue: Issue) {
        self.issues.push(issue);
    }

    pub fn error_count(&self) -> usize {
        self.issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .count()
    }

    /// A group is complete only when it was not cancelled and has no errors.
    pub fn is_complete(&self) -> bool {
        !self.cancelled && self.error_count() == 0
    }

    /// Fixes the `complete` flag from the current issues.
    pub fn finish(&mut self) {
        self.complete = self.is_complete();
    }
}

/// Progress of the lesson currently being downloaded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    pub group: String,
    pub course: String,
    pub lesson: String,
    pub percent: f32,
    pub completed: u32,
    pub total: u32,
}

/// Drops credentials, query and fragment so a URL is safe to log or report.
pub fn redact_url(url: &str) -> String {
    let cut = url.find(['?', '#']).map_or(url, |i| &url[..i]);
    match cut.split_once("://") {
        Some((scheme, rest)) => {
            let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
            let host = authority.rsplit('@').next().unwrap_or(authority);
            format!("{scheme}://{host}{path}")
        }
        None => cut.to_string(),
    }
}

fn url_host(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = host.split(':').next()?;
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redact_drops_query_fragment_and_credentials() {
        assert_eq!(
            redact_url("https://stream.mux.com/abc.m3u8?token=SECRET#x"),
            "https://stream.mux.com/abc.m3u8"
        );
        assert_eq!(
            redact_url("https://user:pw@host.com/a/b?q=1"),
            "https://host.com/a/b"
        );
        assert_eq!(redact_url("https://host.com"), "https://host.com");
        assert_eq!(redact_url("not a url?x=1"), "not a url");
    }

    #[test]
    fn issue_redacts_url_and_derives_host() {
        let i = Issue::error(
            IssueKind::Video,
            Some("01 - Intro"),
            Some("https://Stream.Mux.com:443/abc.m3u8?token=SECRET"),
            "boom",
        );
        assert_eq!(i.host.as_deref(), Some("stream.mux.com"));
        assert_eq!(
            i.url.as_deref(),
            Some("https://Stream.Mux.com:443/abc.m3u8")
        );
        assert!(!serde_json::to_string(&i).unwrap().contains("SECRET"));
    }

    #[test]
    fn issue_serializes_with_snake_case_kind_and_camel_case_fields() {
        let i = Issue::warning(IssueKind::VideoNotResolved, None, None, "m");
        assert_eq!(
            serde_json::to_value(&i).unwrap(),
            json!({"lesson":null,"kind":"video_not_resolved","severity":"warning",
                   "host":null,"url":null,"message":"m"})
        );
    }

    #[test]
    fn complete_requires_no_errors_and_no_cancel() {
        let mut r = DownloadReport::new("skool", "g");
        r.push(Issue::warning(IssueKind::EmptyLesson, Some("l"), None, "w"));
        r.finish();
        assert!(r.complete, "warnings do not block completion");
        r.push(Issue::error(IssueKind::Resource, Some("l"), None, "e"));
        r.finish();
        assert!(!r.complete);
        assert_eq!(r.error_count(), 1);
        let mut c = DownloadReport::new("skool", "g");
        c.cancelled = true;
        c.finish();
        assert!(!c.complete);
    }

    #[test]
    fn report_and_progress_shapes_are_camel_case() {
        let mut r = DownloadReport::new("skool", "g");
        r.total_lessons = 3;
        r.finish();
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(
            v,
            json!({"platform":"skool","group":"g","cancelled":false,"totalLessons":3,
                   "completedLessons":0,"complete":true,"issues":[]})
        );
        let p = DownloadProgress {
            group: "g".into(),
            course: "c".into(),
            lesson: "l".into(),
            percent: 12.5,
            completed: 1,
            total: 4,
        };
        assert_eq!(
            serde_json::to_value(&p).unwrap(),
            json!({"group":"g","course":"c","lesson":"l","percent":12.5,"completed":1,"total":4})
        );
    }
}
