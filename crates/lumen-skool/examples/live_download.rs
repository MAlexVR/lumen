//! Manual live download of ONE course (hits the network, writes files).
//! Reads the cookie header from the `SKOOL_COOKIE` environment variable only.
//!
//!     SKOOL_COOKIE='auth_token=...; ...' cargo run -p lumen-skool --example live_download \
//!         <group-slug> <course-short-name> <output-dir> [sidecar-dir]
//!
//! `sidecar-dir` is the folder holding `yt-dlp` and `ffmpeg` (default: PATH).

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use lumen_core::{
    CancelFlag, CourseProgressSink, DownloadOptions, DownloadProgress, DownloadReport,
};
use lumen_download::YtDlp;
use lumen_skool::client::SkoolClient;
use lumen_skool::downloader::{GroupDownloader, SkoolSource};
use lumen_skool::parse::{CourseListing, CourseTree, ModuleDetail};
use lumen_skool::session::normalize_cookie_header;
use lumen_skool::SkoolError;

/// Delegates to the real client but lists only the chosen course.
struct OneCourse {
    client: SkoolClient,
    only: String,
}

#[async_trait]
impl SkoolSource for OneCourse {
    async fn classroom(&self, slug: &str) -> Result<CourseListing, SkoolError> {
        let mut listing = self.client.classroom(slug).await?;
        listing.courses.retain(|c| c.short_name == self.only);
        Ok(listing)
    }
    async fn course(&self, slug: &str, course: &str) -> Result<CourseTree, SkoolError> {
        self.client.course(slug, course).await
    }
    async fn module(&self, slug: &str, course: &str, id: &str) -> Result<ModuleDetail, SkoolError> {
        self.client.module(slug, course, id).await
    }
    async fn fetch_file(
        &self,
        url: &str,
        dest_base: &Path,
    ) -> Result<lumen_skool::client::FetchedFile, SkoolError> {
        self.client.fetch_file(url, dest_base).await
    }
}

struct Printer;

impl CourseProgressSink for Printer {
    fn on_progress(&self, p: &DownloadProgress) {
        if p.percent >= 100.0 {
            println!(
                "  [{}/{}] {} / {}",
                p.completed, p.total, p.course, p.lesson
            );
        }
    }
    fn on_report(&self, _: &DownloadReport) {}
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(slug), Some(course), Some(out)) = (args.next(), args.next(), args.next()) else {
        return Err(
            "usage: live_download <group-slug> <course-short-name> <output-dir> [sidecar-dir]"
                .into(),
        );
    };
    let sidecar = args.next().map(PathBuf::from);
    let raw = std::env::var("SKOOL_COOKIE").map_err(|_| "set SKOOL_COOKIE")?;
    let source = OneCourse {
        client: SkoolClient::new(&normalize_cookie_header(&raw)?)?,
        only: course,
    };
    let group = source
        .client
        .groups()
        .await?
        .into_iter()
        .find(|g| g.slug == slug)
        .ok_or("unknown group slug")?;
    let videos = YtDlp::locate(sidecar.as_deref());
    let cancel = CancelFlag::new();
    let report = GroupDownloader {
        source: &source,
        videos: &videos,
        sink: &Printer,
        cancel: &cancel,
        options: DownloadOptions {
            max_parallel_lessons: 2,
        },
    }
    .run(&group, Path::new(&out))
    .await?;
    println!(
        "lessons {}/{} complete={} issues={}",
        report.completed_lessons,
        report.total_lessons,
        report.complete,
        report.issues.len()
    );
    for i in &report.issues {
        println!("  {:?} {:?}: {}", i.severity, i.lesson, i.message);
    }
    Ok(())
}
