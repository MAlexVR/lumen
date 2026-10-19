//! Group folder bookkeeping: `download-report.json` and the `.complete` marker.

use std::fs;
use std::io;
use std::path::Path;

use lumen_core::{DownloadReport, Issue, IssueKind};

pub const REPORT_FILE: &str = "download-report.json";
pub const COMPLETE_MARKER: &str = ".complete";

pub fn write_report(group_dir: &Path, report: &DownloadReport) -> io::Result<()> {
    let json = serde_json::to_string_pretty(report).map_err(io::Error::other)?;
    fs::write(group_dir.join(REPORT_FILE), json)
}

/// Creates the `.complete` marker, or removes a stale one.
pub fn set_complete(group_dir: &Path, complete: bool) -> io::Result<()> {
    let marker = group_dir.join(COMPLETE_MARKER);
    if complete {
        fs::write(marker, "done")
    } else {
        match fs::remove_file(marker) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

/// Closes a downloaded folder: writes `download-report.json`, then creates the
/// `.complete` marker only for a finished run without error issues (a stale
/// marker is removed when errors remain; a cancelled run leaves it untouched).
/// I/O failures are never swallowed: they become `Course` error issues and the
/// report is re-evaluated. Returns after `report.complete` is final.
pub fn finalize_folder(dir: &Path, report: &mut DownloadReport) {
    report.finish();
    if let Err(e) = write_report(dir, report) {
        report.push(Issue::error(
            IssueKind::Course,
            None,
            None,
            format!("cannot write the download report: {e}"),
        ));
        report.finish();
    }
    if !report.cancelled {
        if let Err(e) = set_complete(dir, report.complete) {
            report.push(Issue::error(
                IssueKind::Course,
                None,
                None,
                format!("cannot update the completion marker: {e}"),
            ));
            report.finish();
            // The marker may have been left behind; never leave it on a failed run.
            let _ = set_complete(dir, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_core::{Issue, IssueKind};

    #[test]
    fn report_is_written_as_camel_case_json() {
        let tmp = tempfile::tempdir().unwrap();
        let mut r = DownloadReport::new("skool", "G");
        r.push(Issue::error(
            IssueKind::Video,
            Some("l"),
            Some("https://h.test/a?token=SECRET"),
            "boom",
        ));
        r.finish();
        write_report(tmp.path(), &r).unwrap();
        let text = fs::read_to_string(tmp.path().join(REPORT_FILE)).unwrap();
        assert!(text.contains("\"totalLessons\""));
        assert!(text.contains("\"video\""));
        assert!(!text.contains("SECRET"));
        let back: DownloadReport = serde_json::from_str(&text).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn marker_is_created_and_removed_idempotently() {
        let tmp = tempfile::tempdir().unwrap();
        set_complete(tmp.path(), false).unwrap();
        set_complete(tmp.path(), true).unwrap();
        assert_eq!(
            fs::read_to_string(tmp.path().join(COMPLETE_MARKER)).unwrap(),
            "done"
        );
        set_complete(tmp.path(), false).unwrap();
        assert!(!tmp.path().join(COMPLETE_MARKER).exists());
    }

    #[test]
    fn finalize_marks_a_clean_run_complete_and_writes_the_report() {
        let tmp = tempfile::tempdir().unwrap();
        let mut r = DownloadReport::new("hotmart", "C");
        r.push(Issue::warning(IssueKind::EmptyLesson, Some("l"), None, "w"));
        finalize_folder(tmp.path(), &mut r);
        assert!(r.complete);
        assert!(tmp.path().join(COMPLETE_MARKER).is_file());
        let back: DownloadReport =
            serde_json::from_str(&fs::read_to_string(tmp.path().join(REPORT_FILE)).unwrap())
                .unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn finalize_removes_a_stale_marker_when_errors_remain() {
        let tmp = tempfile::tempdir().unwrap();
        set_complete(tmp.path(), true).unwrap();
        let mut r = DownloadReport::new("hotmart", "C");
        r.push(Issue::error(IssueKind::Video, Some("l"), None, "boom"));
        finalize_folder(tmp.path(), &mut r);
        assert!(!r.complete);
        assert!(!tmp.path().join(COMPLETE_MARKER).exists());
    }

    #[test]
    fn finalize_leaves_the_marker_alone_for_a_cancelled_run() {
        let tmp = tempfile::tempdir().unwrap();
        set_complete(tmp.path(), true).unwrap();
        let mut r = DownloadReport::new("hotmart", "C");
        r.cancelled = true;
        finalize_folder(tmp.path(), &mut r);
        assert!(!r.complete);
        assert!(
            tmp.path().join(COMPLETE_MARKER).is_file(),
            "cancel keeps the old state"
        );
    }

    #[test]
    fn finalize_reports_io_failures_as_errors_instead_of_swallowing_them() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("gone");
        let mut r = DownloadReport::new("hotmart", "C");
        finalize_folder(&missing, &mut r);
        assert!(!r.complete);
        assert!(r
            .issues
            .iter()
            .any(|i| i.kind == IssueKind::Course && i.message.contains("download report")));
    }
}
