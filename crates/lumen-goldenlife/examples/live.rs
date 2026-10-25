//! Manual live check against the academy (hits the network). The credentials
//! file (`user=<email>` and `password=<password>` lines, e.g. the git-ignored
//! `secure/golden`) is passed explicitly; nothing secret is ever printed.
//!
//!     live <credentials-file> survey
//!     live <credentials-file> download <output-dir> <course-id>...
//!
//! `survey` walks every course, section and lesson (read-only) and checks
//! that each Panda playlist answers; `download` runs the real downloader.

use std::path::{Path, PathBuf};
use std::time::Duration;

use lumen_core::{
    CancelFlag, CourseProgressSink, DownloadOptions, DownloadProgress, DownloadReport, Severity,
};
use lumen_download::YtDlp;
use lumen_goldenlife::client::BROWSER_UA;
use lumen_goldenlife::parse::EmbedKind;
use lumen_goldenlife::{
    ClientConfig, CourseDownloader, GoldenlifeApi, GoldenlifeClient, Login, Retry,
};

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

fn read_login(path: &Path) -> Result<Login, Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(path)?;
    let value = |keys: &[&str]| {
        text.lines().find_map(|line| {
            let (key, value) = line.split_once(['=', ':'])?;
            keys.contains(&key.trim().to_lowercase().as_str())
                .then(|| value.trim().to_string())
        })
    };
    let email = value(&["user", "email"]).ok_or("no user= line")?;
    let password = value(&["password", "pass"]).ok_or("no password line")?;
    Ok(Login::new(&email, &password)?)
}

async fn playlist_ok(http: &reqwest::Client, url: &str, referer: &str) -> Result<(), String> {
    let response = http
        .get(url)
        .header("referer", referer)
        .header("user-agent", BROWSER_UA)
        .send()
        .await
        .map_err(|e| e.without_url().to_string())?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| e.without_url().to_string())?;
    if status.is_success() && body.starts_with("#EXTM3U") {
        Ok(())
    } else {
        Err(format!("HTTP {status}"))
    }
}

async fn survey(client: &GoldenlifeClient) -> Result<(), Box<dyn std::error::Error>> {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;
    let courses = client.courses().await?;
    println!("courses: {}", courses.len());
    let (mut empty, mut sections, mut lessons) = (Vec::new(), 0, 0);
    let (mut panda, mut youtube, mut text_only, mut missing, mut attachments) = (0, 0, 0, 0, 0);
    let mut problems: Vec<String> = Vec::new();
    for course in &courses {
        let outline = client.outline(&course.id).await?;
        if outline.sections.is_empty() {
            empty.push(format!("{} ({})", course.title, course.id));
            continue;
        }
        sections += outline.sections.len();
        for section in &outline.sections {
            for lesson in &section.lessons {
                lessons += 1;
                match client.lesson(&lesson.id).await {
                    Err(e) => problems.push(format!("lesson {}: {e}", lesson.id)),
                    Ok(page) => {
                        attachments += page.attachments.len();
                        if page.video_missing {
                            missing += 1;
                        }
                        if page.main_video.is_none() && page.embeds.is_empty() {
                            text_only += 1;
                        }
                        for embed in page.main_video.iter().chain(&page.embeds) {
                            match embed.kind {
                                EmbedKind::Panda => {
                                    panda += 1;
                                    if let Err(e) =
                                        playlist_ok(&http, &embed.url, &client.referer()).await
                                    {
                                        problems
                                            .push(format!("lesson {} playlist: {e}", lesson.id));
                                    }
                                }
                                EmbedKind::YouTube => youtube += 1,
                                EmbedKind::Other => {
                                    problems.push(format!("lesson {}: unknown player", lesson.id));
                                }
                            }
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
        }
    }
    println!("courses without content of their own: {}", empty.len());
    for e in &empty {
        println!("  {e}");
    }
    println!(
        "sections {sections} lessons {lessons} | panda videos {panda} (playlists verified) youtube {youtube} \
         text-only lessons {text_only} no-video-on-platform {missing} attachments {attachments}"
    );
    println!("problems: {}", problems.len());
    for p in &problems {
        println!("  {p}");
    }
    Ok(())
}

async fn download(
    client: &GoldenlifeClient,
    out: &Path,
    ids: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    let courses = client.courses().await?;
    let ytdlp = YtDlp::locate(None);
    let cancel = CancelFlag::new();
    for id in ids {
        let course = courses
            .iter()
            .find(|c| &c.id == id)
            .ok_or_else(|| format!("unknown course {id}"))?;
        println!("== {} ({})", course.title, course.id);
        let report = CourseDownloader {
            api: client,
            videos: &ytdlp,
            sink: &Printer,
            cancel: &cancel,
            options: DownloadOptions {
                max_parallel_lessons: 2,
            },
            retry: Retry::default(),
        }
        .run(course, out)
        .await?;
        println!(
            "   {}/{} lessons, complete={}, errors={}, warnings={}",
            report.completed_lessons,
            report.total_lessons,
            report.complete,
            report.error_count(),
            report
                .issues
                .iter()
                .filter(|i| i.severity == Severity::Warning)
                .count()
        );
        for issue in &report.issues {
            println!(
                "   {:?} {:?} {} - {}",
                issue.severity,
                issue.kind,
                issue.lesson.as_deref().unwrap_or("-"),
                issue.message
            );
        }
    }
    Ok(())
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let usage = "usage: live <credentials-file> survey | download <out> <course-id>...";
    let credentials = PathBuf::from(args.next().ok_or(usage)?);
    let command = args.next().ok_or(usage)?;
    let client =
        GoldenlifeClient::login(&ClientConfig::default(), &read_login(&credentials)?).await?;
    client.validate().await?;
    println!("login ok");
    match command.as_str() {
        "survey" => survey(&client).await,
        "download" => {
            let out = PathBuf::from(args.next().ok_or(usage)?);
            let ids: Vec<String> = args.collect();
            download(&client, &out, &ids).await
        }
        _ => Err(usage.into()),
    }
}
