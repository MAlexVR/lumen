//! Manual live check against Hotmart (hits the network). Reads the full cookie
//! header from `HOTMART_COOKIE` (it must contain `hmVlcIntegration`); never
//! reads files.
//!
//!     live list
//!     live download <product-id> <output-dir> [sidecar-dir]
//!
//! `sidecar-dir` is the folder holding `yt-dlp` and `ffmpeg` (default: PATH).

use std::path::{Path, PathBuf};

use lumen_core::{
    CancelFlag, CourseProgressSink, DownloadOptions, DownloadProgress, DownloadReport,
};
use lumen_download::YtDlp;
use lumen_hotmart::client::HotmartClient;
use lumen_hotmart::credentials::{normalize_cookies, normalize_token, Credentials};
use lumen_hotmart::downloader::{CourseDownloader, Retry};

struct Printer;

impl CourseProgressSink for Printer {
    fn on_progress(&self, p: &DownloadProgress) {
        if p.percent >= 100.0 {
            println!("  [{}/{}] {}", p.completed, p.total, p.lesson);
        }
    }
    fn on_report(&self, _: &DownloadReport) {}
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let command = args
        .next()
        .ok_or("usage: live list | download <id> <out> [sidecar]")?;
    let raw = std::env::var("HOTMART_COOKIE").map_err(|_| "set HOTMART_COOKIE")?;
    let cookies = normalize_cookies(&raw)?.ok_or("empty cookie header")?;
    let token = normalize_token(&cookies)?;
    let client = HotmartClient::new(Credentials::new(&token, Some(&cookies))?)?;
    let purchases = client.courses().await?;
    if command == "list" {
        for c in &purchases.courses {
            println!("{} | {} | {}", c.id, c.slug, c.name);
        }
        println!("({} without members area)", purchases.skipped);
        return Ok(());
    }
    let id = args.next().ok_or("missing product id")?;
    let out = args.next().ok_or("missing output dir")?;
    let sidecar = args.next().map(PathBuf::from);
    let course = purchases
        .courses
        .iter()
        .find(|c| c.id == id)
        .ok_or("unknown product id")?;
    let videos = YtDlp::locate(sidecar.as_deref());
    let cancel = CancelFlag::new();
    let report = CourseDownloader {
        source: &client,
        videos: &videos,
        sink: &Printer,
        cancel: &cancel,
        options: DownloadOptions {
            max_parallel_lessons: 2,
        },
        retry: Retry::default(),
    }
    .run(course, Path::new(&out))
    .await?;
    println!(
        "lessons {}/{} complete={} issues={}",
        report.completed_lessons,
        report.total_lessons,
        report.complete,
        report.issues.len()
    );
    for i in &report.issues {
        println!(
            "  {:?} {:?} [{:?}]: {}",
            i.severity, i.lesson, i.kind, i.message
        );
    }
    Ok(())
}
