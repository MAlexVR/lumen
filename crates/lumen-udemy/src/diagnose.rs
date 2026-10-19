//! The diagnostic ("wizard" backend): with the owner's session it measures what
//! Udemy really returns and produces a sanitized report, so phase 2 (the
//! downloader) is built from real responses instead of guesses. It lists all
//! courses, reads every curriculum and probes a few lectures per course,
//! classifying them (DRM is only detected, never touched). Raw responses are
//! never persisted: only sanitized, truncated samples of the first response of
//! each endpoint shape.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use futures_util::StreamExt;
use lumen_core::{CancelFlag, Issue, IssueKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::classify::{classify_item, verdict, ClassCounts, LectureClass, Verdict};
use crate::client::UdemyClient;
use crate::error::UdemyError;
use crate::parse::{
    parse_courses_page, parse_curriculum_page, parse_lecture_detail, CourseSummary, CurriculumItem,
    ItemKind,
};
use crate::sanitize::{sanitize, scrub_text, truncate_arrays, SanitizeOptions};

pub const ENDPOINT_COURSES: &str = "subscribedCourses";
pub const ENDPOINT_CURRICULUM: &str = "curriculumItems";
pub const ENDPOINT_LECTURE: &str = "lectureDetail";
/// Arrays in samples keep this many elements.
const SAMPLE_ARRAY_LIMIT: usize = 3;

/// Dotted paths whose presence the diagnostic counts, per response shape.
/// This is how each assumed field gets verified (or refuted).
const COURSE_FIELDS: [&str; 6] = [
    "id",
    "title",
    "image_480x270",
    "url",
    "num_published_lectures",
    "visible_instructors",
];
const CURRICULUM_FIELDS: [&str; 14] = [
    "title",
    "object_index",
    "is_published",
    "sort_order",
    "description",
    "supplementary_assets",
    "asset",
    "asset.asset_type",
    "asset.stream_urls",
    "asset.download_urls",
    "asset.media_sources",
    "asset.media_license_token",
    "asset.course_is_drmed",
    "asset.is_external",
];
const LECTURE_FIELDS: [&str; 12] = [
    "description",
    "download_url",
    "asset",
    "asset.asset_type",
    "asset.stream_urls",
    "asset.download_urls",
    "asset.captions",
    "asset.media_sources",
    "asset.media_license_token",
    "asset.course_is_drmed",
    "asset.body",
    "asset.external_url",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Step {
    Session,
    Courses,
    Curriculum,
    Lectures,
    Finish,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnoseProgress {
    pub step: Step,
    pub current: u32,
    pub total: u32,
    pub message: String,
}

pub trait DiagnoseSink: Send + Sync {
    fn progress(&self, progress: &DiagnoseProgress);
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DiagnoseOptions {
    pub max_courses: Option<usize>,
    pub sample_lectures_per_course: usize,
    pub include_titles: bool,
}

impl Default for DiagnoseOptions {
    fn default() -> Self {
        Self {
            max_courses: None,
            sample_lectures_per_course: 5,
            include_titles: false,
        }
    }
}

/// Politeness settings (zero delay in tests).
#[derive(Debug, Clone)]
pub struct Pacing {
    /// Pause before each lecture-detail request.
    pub detail_delay: Duration,
    pub course_concurrency: usize,
}

impl Default for Pacing {
    fn default() -> Self {
        Self {
            detail_delay: Duration::from_millis(300),
            course_concurrency: 2,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RunMeta {
    pub app_version: String,
    pub generated_at: String,
    /// Credential values scrubbed from everything that is written.
    pub secrets: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Complete,
    Cancelled,
    /// Stopped early (the session expired or Udemy blocked the requests).
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EndpointReport {
    pub name: String,
    /// `ok`, `http_401`, `http_403`, `http_404`, `parse_error`, `network`, `not_tested`...
    pub status: String,
    pub http_status: Option<u16>,
    /// True only when every request to it succeeded.
    pub verified: bool,
    pub note: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct TypeCounts {
    pub video: u32,
    pub article: u32,
    pub quiz: u32,
    pub other: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CourseReport {
    pub id: String,
    /// The real title only with `includeTitles`, else a length placeholder.
    pub title: String,
    pub lectures: u32,
    pub types: TypeCounts,
    pub probed: u32,
    pub classification: ClassCounts,
    pub verdict: Verdict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct VerdictCounts {
    pub downloadable: u32,
    pub partial: u32,
    pub none: u32,
    pub unknown: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    pub courses: u32,
    pub lectures: u32,
    pub probed: u32,
    pub classification: ClassCounts,
    pub verdicts: VerdictCounts,
}

/// How many inspected responses carried a field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct FieldStat {
    pub present: u32,
    pub total: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UdemyDiagnostic {
    pub generated_at: String,
    pub app_version: String,
    pub account_kind: String,
    pub status: RunStatus,
    pub options: DiagnoseOptions,
    pub endpoints: Vec<EndpointReport>,
    pub courses: Vec<CourseReport>,
    pub totals: Totals,
    pub issues: Vec<Issue>,
    /// Per response shape, per assumed field: how often it was present.
    pub fields: BTreeMap<String, BTreeMap<String, FieldStat>>,
    /// Sanitized, truncated first response of each endpoint shape.
    pub samples: BTreeMap<String, Value>,
}

impl UdemyDiagnostic {
    /// Writes pretty JSON privately (0600 on Unix) after a last scrub of every
    /// known secret from the serialized text. Atomic through a temp file.
    pub fn write_to(&self, path: &Path, secrets: &[String]) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let text = scrub_text(&json, secrets);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = std::path::PathBuf::from(tmp);
        let result = (|| {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            let mut file = options.open(&tmp)?;
            #[cfg(unix)]
            std::fs::set_permissions(&tmp, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&tmp, path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }
}

/// `YYYY-MM-DDTHH:MM:SSZ` for a Unix timestamp.
pub fn iso8601_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Indices (into `lectures`) of up to `n` lectures covering the types: the
/// first video, the first article, the first with resources, the last one,
/// then evenly spaced fillers.
pub fn pick_sample(lectures: &[&CurriculumItem], n: usize) -> Vec<usize> {
    let mut picked: Vec<usize> = Vec::new();
    let asset_is = |l: &CurriculumItem, kind: &str| {
        l.asset
            .as_ref()
            .and_then(|a| a.asset_type.as_deref())
            .is_some_and(|t| t.eq_ignore_ascii_case(kind))
    };
    let firsts = [
        lectures.iter().position(|l| asset_is(l, "video")),
        lectures.iter().position(|l| asset_is(l, "article")),
        lectures.iter().position(|l| !l.supplementary.is_empty()),
        lectures.len().checked_sub(1),
    ];
    for i in firsts.into_iter().flatten() {
        if !picked.contains(&i) {
            picked.push(i);
        }
    }
    let rest: Vec<usize> = (0..lectures.len())
        .filter(|i| !picked.contains(i))
        .collect();
    let need = n.saturating_sub(picked.len().min(n));
    let take = need.min(rest.len());
    if take > 0 {
        let step = rest.len() as f64 / take as f64;
        for k in 0..take {
            picked.push(rest[(k as f64 * step) as usize]);
        }
    }
    picked.truncate(n);
    picked
}

fn has_path(v: &Value, path: &str) -> bool {
    let mut cur = v;
    for key in path.split('.') {
        match cur.get(key) {
            Some(next) => cur = next,
            None => return false,
        }
    }
    !cur.is_null()
}

type Tally = BTreeMap<String, FieldStat>;

fn tally(into: &mut Tally, paths: &[&str], objects: &[&Value]) {
    for path in paths {
        let stat = into.entry((*path).to_string()).or_default();
        for o in objects {
            stat.total += 1;
            stat.present += u32::from(has_path(o, path));
        }
    }
}

fn merge_tally(into: &mut Tally, from: &Tally) {
    for (k, s) in from {
        let e = into.entry(k.clone()).or_default();
        e.present += s.present;
        e.total += s.total;
    }
}

#[derive(Default)]
struct EndpointTracker {
    ok: u32,
    failures: Vec<String>,
}

impl EndpointTracker {
    fn record(&mut self, result: &Result<(), UdemyError>) {
        match result {
            Ok(()) => self.ok += 1,
            Err(e) => self.failures.push(e.status_label()),
        }
    }

    fn report(&self, name: &str) -> EndpointReport {
        let total = self.ok as usize + self.failures.len();
        let mut labels = self.failures.clone();
        labels.sort();
        labels.dedup();
        let first = labels.first().cloned();
        let (status, note) = match (&first, self.ok) {
            (None, 0) => ("not_tested".to_string(), "never requested".to_string()),
            (None, n) => ("ok".to_string(), format!("{n} request(s) succeeded")),
            (Some(label), _) => (
                label.clone(),
                format!(
                    "{} of {total} request(s) failed ({})",
                    self.failures.len(),
                    labels.join(", ")
                ),
            ),
        };
        let http_status = match &first {
            Some(label) => label.strip_prefix("http_").and_then(|s| s.parse().ok()),
            None => (self.ok > 0).then_some(200),
        };
        EndpointReport {
            name: name.to_string(),
            verified: first.is_none() && self.ok > 0,
            status,
            http_status,
            note,
        }
    }
}

struct CourseOutcome {
    report: CourseReport,
    issues: Vec<Issue>,
    curriculum_events: Vec<Result<(), UdemyError>>,
    detail_events: Vec<Result<(), UdemyError>>,
    curriculum_fields: Tally,
    detail_fields: Tally,
    curriculum_sample: Option<Value>,
    detail_sample: Option<Value>,
}

struct Shared<'a> {
    client: &'a UdemyClient,
    opts: &'a DiagnoseOptions,
    pacing: &'a Pacing,
    sink: &'a dyn DiagnoseSink,
    cancel: &'a CancelFlag,
    abort: AtomicBool,
    planned: AtomicU32,
    probed: AtomicU32,
}

fn placeholder_title(title: Option<&str>, include: bool) -> String {
    match (title, include) {
        (Some(t), true) => t.to_string(),
        (Some(t), false) => format!("<title len={}>", t.chars().count()),
        (None, _) => "<no title>".to_string(),
    }
}

fn lecture_label(item: &CurriculumItem) -> String {
    format!("lecture {}", item.id)
}

fn is_session_loss(e: &UdemyError) -> bool {
    matches!(e, UdemyError::Unauthorized | UdemyError::BotChallenge)
}

async fn process_course(shared: &Shared<'_>, course: CourseSummary) -> Option<CourseOutcome> {
    if shared.cancel.is_cancelled() || shared.abort.load(Ordering::SeqCst) {
        return None;
    }
    let mut out = CourseOutcome {
        report: CourseReport {
            id: course.id.clone(),
            title: placeholder_title(course.title.as_deref(), shared.opts.include_titles),
            lectures: 0,
            types: TypeCounts::default(),
            probed: 0,
            classification: ClassCounts::default(),
            verdict: Verdict::Unknown,
        },
        issues: Vec::new(),
        curriculum_events: Vec::new(),
        detail_events: Vec::new(),
        curriculum_fields: Tally::new(),
        detail_fields: Tally::new(),
        curriculum_sample: None,
        detail_sample: None,
    };
    let course_label = format!("course {}", course.id);
    let client = shared.client;
    let url = match client.endpoints().curriculum_url(&course.id) {
        Ok(u) => u,
        Err(e) => {
            out.issues.push(Issue::error(
                IssueKind::Course,
                Some(&course_label),
                None,
                e.to_string(),
            ));
            return Some(out);
        }
    };
    let pages = client.pages(&url).await;
    out.curriculum_events
        .extend(pages.values.iter().map(|_| Ok(())));
    if let Some(e) = &pages.error {
        out.curriculum_events.push(Err(e.clone()));
        out.issues.push(Issue::error(
            IssueKind::Course,
            Some(&course_label),
            None,
            format!("could not read the curriculum: {}", e.status_label()),
        ));
        if is_session_loss(e) {
            shared.abort.store(true, Ordering::SeqCst);
        }
    }
    out.curriculum_sample = pages.values.first().cloned();
    let mut items: Vec<CurriculumItem> = Vec::new();
    for page in &pages.values {
        match parse_curriculum_page(page) {
            Ok(p) => {
                if p.skipped > 0 {
                    out.issues.push(Issue::warning(
                        IssueKind::Course,
                        Some(&course_label),
                        None,
                        format!("{} curriculum item(s) could not be read", p.skipped),
                    ));
                }
                items.extend(p.items);
            }
            Err(e) => out.issues.push(Issue::error(
                IssueKind::Course,
                Some(&course_label),
                None,
                format!("unexpected curriculum page: {e}"),
            )),
        }
    }
    let raw_lectures: Vec<&Value> = pages
        .values
        .iter()
        .filter_map(|p| p.get("results").and_then(Value::as_array))
        .flatten()
        .filter(|i| i.get("_class").and_then(Value::as_str) == Some("lecture"))
        .collect();
    tally(
        &mut out.curriculum_fields,
        &CURRICULUM_FIELDS,
        &raw_lectures,
    );

    let lectures: Vec<&CurriculumItem> = items
        .iter()
        .filter(|i| i.kind == ItemKind::Lecture)
        .collect();
    out.report.lectures = lectures.len() as u32;
    for l in &lectures {
        match l
            .asset
            .as_ref()
            .and_then(|a| a.asset_type.as_deref())
            .map(str::to_lowercase)
            .as_deref()
        {
            Some("video") => out.report.types.video += 1,
            Some("article") => out.report.types.article += 1,
            _ => out.report.types.other += 1,
        }
    }
    for item in &items {
        if matches!(item.kind, ItemKind::Quiz | ItemKind::Practice) {
            out.report.types.quiz += 1;
            out.report.classification.add(LectureClass::Quiz);
        }
    }

    let sample = pick_sample(&lectures, shared.opts.sample_lectures_per_course);
    shared
        .planned
        .fetch_add(sample.len() as u32, Ordering::SeqCst);
    for idx in sample {
        if shared.cancel.is_cancelled() || shared.abort.load(Ordering::SeqCst) {
            break;
        }
        let item = lectures[idx];
        if !shared.pacing.detail_delay.is_zero() {
            tokio::time::sleep(shared.pacing.detail_delay).await;
        }
        let label = lecture_label(item);
        let detail_url = match client.endpoints().lecture_url(&course.id, &item.id) {
            Ok(u) => u,
            Err(e) => {
                out.issues.push(Issue::warning(
                    IssueKind::VideoNotResolved,
                    Some(&label),
                    None,
                    e.to_string(),
                ));
                continue;
            }
        };
        let response = client.get_json(&detail_url).await;
        let current = shared.probed.fetch_add(1, Ordering::SeqCst) + 1;
        shared.sink.progress(&DiagnoseProgress {
            step: Step::Lectures,
            current,
            total: shared.planned.load(Ordering::SeqCst).max(current),
            message: format!("probing {label} of course {}", course.id),
        });
        out.report.probed += 1;
        match response {
            Ok(raw) => {
                out.detail_events.push(Ok(()));
                tally(&mut out.detail_fields, &LECTURE_FIELDS, &[&raw]);
                if out.detail_sample.is_none() {
                    out.detail_sample = Some(raw.clone());
                }
                let detail = parse_lecture_detail(&raw).ok();
                let asset = detail
                    .as_ref()
                    .and_then(|d| d.asset.as_ref())
                    .or(item.asset.as_ref());
                let c = classify_item(&item.kind, asset, &item.supplementary);
                if c.class == LectureClass::Drm {
                    out.issues.push(Issue::warning(
                        IssueKind::VideoNotResolved,
                        Some(&label),
                        None,
                        format!("drm_protected: {}", c.reason.clone().unwrap_or_default()),
                    ));
                }
                out.report.classification.add(c.class);
            }
            Err(e) => {
                out.detail_events.push(Err(e.clone()));
                out.report.classification.add(LectureClass::Unknown);
                if is_session_loss(&e) {
                    shared.abort.store(true, Ordering::SeqCst);
                    out.issues.push(Issue::error(
                        IssueKind::Course,
                        Some(&label),
                        None,
                        format!(
                            "session expired or blocked during the run: {}",
                            e.status_label()
                        ),
                    ));
                } else {
                    out.issues.push(Issue::warning(
                        IssueKind::VideoNotResolved,
                        Some(&label),
                        None,
                        format!("could not fetch lecture detail: {}", e.status_label()),
                    ));
                }
            }
        }
    }
    out.report.verdict = verdict(&out.report.classification);
    Some(out)
}

/// Runs the whole diagnostic. Only a failure of the first session check is an
/// `Err`; everything else is recorded in the result.
pub async fn run(
    client: &UdemyClient,
    opts: &DiagnoseOptions,
    pacing: &Pacing,
    sink: &dyn DiagnoseSink,
    cancel: &CancelFlag,
    meta: &RunMeta,
) -> Result<UdemyDiagnostic, UdemyError> {
    let emit = |step: Step, current: u32, total: u32, message: String| {
        sink.progress(&DiagnoseProgress {
            step,
            current,
            total,
            message,
        });
    };
    let mut courses_ep = EndpointTracker::default();
    let mut curriculum_ep = EndpointTracker::default();
    let mut detail_ep = EndpointTracker::default();
    let mut issues: Vec<Issue> = Vec::new();
    let mut samples: BTreeMap<String, Value> = BTreeMap::new();
    let mut fields: BTreeMap<String, Tally> = BTreeMap::new();
    let sanitize_opts = SanitizeOptions {
        include_titles: opts.include_titles,
        secrets: meta.secrets.clone(),
    };
    let sample_of =
        |raw: &Value| sanitize(&truncate_arrays(raw, SAMPLE_ARRAY_LIMIT), &sanitize_opts);

    emit(Step::Session, 0, 1, "verifying the session".into());
    let session = client.get_json(&client.endpoints().courses_url(1)).await;
    courses_ep.record(&session.as_ref().map(|_| ()).map_err(Clone::clone));
    session?;
    emit(Step::Session, 1, 1, "session verified".into());

    emit(Step::Courses, 0, 0, "listing courses".into());
    let pages = client.pages(&client.endpoints().courses_url(100)).await;
    courses_ep.record(&Ok(()));
    let mut courses: Vec<CourseSummary> = Vec::new();
    let mut course_fields = Tally::new();
    for page in &pages.values {
        match parse_courses_page(page) {
            Ok(p) => {
                if p.skipped > 0 {
                    issues.push(Issue::warning(
                        IssueKind::Course,
                        None,
                        None,
                        format!("{} course(s) in the list could not be read", p.skipped),
                    ));
                }
                let raws: Vec<&Value> = page
                    .get("results")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter(|x| x.is_object()).collect())
                    .unwrap_or_default();
                tally(&mut course_fields, &COURSE_FIELDS, &raws);
                courses.extend(p.courses);
            }
            Err(e) => issues.push(Issue::error(
                IssueKind::Course,
                None,
                None,
                format!("unexpected courses page: {e}"),
            )),
        }
    }
    if let Some(first) = pages.values.first() {
        samples.insert(ENDPOINT_COURSES.into(), sample_of(first));
    }
    fields.insert(ENDPOINT_COURSES.into(), course_fields);
    if let Some(e) = &pages.error {
        courses_ep.record(&Err(e.clone()));
        issues.push(Issue::error(
            IssueKind::Course,
            None,
            None,
            format!("could not list every course: {}", e.status_label()),
        ));
    }
    if let Some(max) = opts.max_courses {
        courses.truncate(max);
    }
    let total = courses.len() as u32;
    emit(
        Step::Courses,
        total,
        total,
        format!("found {total} course(s)"),
    );

    emit(Step::Curriculum, 0, total, "reading curriculums".into());
    let shared = Shared {
        client,
        opts,
        pacing,
        sink,
        cancel,
        abort: AtomicBool::new(false),
        planned: AtomicU32::new(0),
        probed: AtomicU32::new(0),
    };
    let mut stream = futures_util::stream::iter(courses.iter().cloned())
        .map(|c| process_course(&shared, c))
        .buffer_unordered(pacing.course_concurrency.max(1));
    let mut reports: Vec<CourseReport> = Vec::new();
    let mut curriculum_fields = Tally::new();
    let mut detail_fields = Tally::new();
    let mut done = 0u32;
    while let Some(outcome) = stream.next().await {
        let Some(o) = outcome else { continue };
        done += 1;
        emit(
            Step::Curriculum,
            done,
            total,
            format!("course {} read", o.report.id),
        );
        o.curriculum_events
            .iter()
            .for_each(|r| curriculum_ep.record(r));
        o.detail_events.iter().for_each(|r| detail_ep.record(r));
        merge_tally(&mut curriculum_fields, &o.curriculum_fields);
        merge_tally(&mut detail_fields, &o.detail_fields);
        if let Some(s) = &o.curriculum_sample {
            samples
                .entry(ENDPOINT_CURRICULUM.into())
                .or_insert_with(|| sample_of(s));
        }
        if let Some(s) = &o.detail_sample {
            samples
                .entry(ENDPOINT_LECTURE.into())
                .or_insert_with(|| sample_of(s));
        }
        issues.extend(o.issues);
        reports.push(o.report);
    }
    drop(stream);
    fields.insert(ENDPOINT_CURRICULUM.into(), curriculum_fields);
    fields.insert(ENDPOINT_LECTURE.into(), detail_fields);

    reports.sort_by(|a, b| a.id.cmp(&b.id));
    let mut totals = Totals {
        courses: reports.len() as u32,
        lectures: 0,
        probed: 0,
        classification: ClassCounts::default(),
        verdicts: VerdictCounts::default(),
    };
    for r in &reports {
        totals.lectures += r.lectures;
        totals.probed += r.probed;
        totals.classification.merge(&r.classification);
        let slot = match r.verdict {
            Verdict::Downloadable => &mut totals.verdicts.downloadable,
            Verdict::Partial => &mut totals.verdicts.partial,
            Verdict::None => &mut totals.verdicts.none,
            Verdict::Unknown => &mut totals.verdicts.unknown,
        };
        *slot += 1;
    }
    let status = if cancel.is_cancelled() {
        RunStatus::Cancelled
    } else if shared.abort.load(Ordering::SeqCst) {
        RunStatus::Aborted
    } else {
        RunStatus::Complete
    };
    if status == RunStatus::Aborted {
        issues.push(Issue::error(
            IssueKind::Course,
            None,
            None,
            "session expired or blocked: the run stopped early",
        ));
    }
    emit(Step::Finish, total, total, "finished".into());
    Ok(UdemyDiagnostic {
        generated_at: meta.generated_at.clone(),
        app_version: meta.app_version.clone(),
        account_kind: "personal".into(),
        status,
        options: opts.clone(),
        endpoints: vec![
            courses_ep.report(ENDPOINT_COURSES),
            curriculum_ep.report(ENDPOINT_CURRICULUM),
            detail_ep.report(ENDPOINT_LECTURE),
        ],
        courses: reports,
        totals,
        issues,
        fields,
        samples,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{ClientConfig, Endpoints};
    use crate::credentials::Credentials;
    use lumen_download::testsrv::{Recorded, Reply, TestServer};
    use lumen_download::RetryPolicy;
    use serde_json::{json, Value};
    use std::sync::Mutex;

    const TOKEN: &str = "TOKENSECRET1";
    const EMAIL: &str = "ana.owner@example.com";

    #[derive(Default)]
    struct Rec {
        events: Mutex<Vec<DiagnoseProgress>>,
        cancel_at: Option<(Step, CancelFlag)>,
    }

    impl DiagnoseSink for Rec {
        fn progress(&self, p: &DiagnoseProgress) {
            if let Some((step, flag)) = &self.cancel_at {
                if p.step == *step {
                    flag.cancel();
                }
            }
            self.events.lock().unwrap().push(p.clone());
        }
    }

    fn challenge() -> Reply {
        let mut r = Reply::html("<html><title>Just a moment...</title></html>");
        r.status = 403;
        r
    }

    fn with_status(mut r: Reply, status: u16) -> Reply {
        r.status = status;
        r
    }

    fn fixture(text: &str, base: &str) -> String {
        text.replace("https://www.udemy.com", base)
    }

    /// A pretend Udemy: three courses; 1001 has a full curriculum, 1002 hits a
    /// bot challenge, 1003 is gone. Lecture 23 is missing; everything else is
    /// built from the assumed fixtures.
    fn udemy(req: &Recorded, _: usize) -> Reply {
        let host = req.header("host").unwrap();
        let base = format!("http://{host}");
        let p = req.path_and_query.as_str();
        if p.contains("/subscribed-courses/") && p.contains("/lectures/") {
            let id = p
                .split("/lectures/")
                .nth(1)
                .unwrap()
                .split('/')
                .next()
                .unwrap();
            return match id {
                "11" => Reply::json(include_str!("../tests/fixtures/assumed_lecture_video.json")),
                "12" => Reply::json(include_str!(
                    "../tests/fixtures/assumed_lecture_article.json"
                )),
                "21" => Reply::json(include_str!("../tests/fixtures/assumed_lecture_drm.json")),
                "22" => Reply::json(
                    r#"{"id":22,"asset":{"asset_type":"ExternalLink","is_external":true,"external_url":"https://example.com/x"}}"#,
                ),
                _ => Reply::status(404),
            };
        }
        if p.contains("/subscribed-courses/") {
            if p.contains("page=2") {
                return Reply::json(
                    &json!({"count": 3, "next": null, "results": [
                    {"_class": "course", "id": 1003, "title": "Tercero"}]})
                    .to_string(),
                );
            }
            let instructors =
                json!([{"_class": "user", "id": 777, "display_name": "Ana", "email": EMAIL}]);
            if p.contains("page_size=100") {
                return Reply::json(&json!({"count": 3, "next": format!("{base}/api-2.0/users/me/subscribed-courses/?page=2"),
                    "results": [
                        {"_class": "course", "id": 1001, "title": "Curso Uno", "visible_instructors": instructors, "num_published_lectures": 5},
                        {"_class": "course", "id": 1002, "title": "Curso Dos"}]}).to_string());
            }
            return Reply::json(r#"{"count": 3, "next": null, "results": [{"id": 1001}]}"#);
        }
        if p.contains("/courses/1001/subscriber-curriculum-items/") {
            let text = if p.contains("page=2") {
                include_str!("../tests/fixtures/assumed_curriculum_page2.json")
            } else {
                include_str!("../tests/fixtures/assumed_curriculum_page1.json")
            };
            return Reply::json(&fixture(text, &base));
        }
        if p.contains("/courses/1002/") {
            return with_status(
                Reply::json(r#"{"detail":"You do not have permission"}"#),
                403,
            );
        }
        Reply::status(404)
    }

    fn creds() -> Credentials {
        Credentials::from_cookie_text(&format!("client_id=cid12345; access_token={TOKEN}")).unwrap()
    }

    fn client(base: &str) -> UdemyClient {
        UdemyClient::with_config(
            creds(),
            ClientConfig {
                endpoints: Endpoints::under(base),
                retry: RetryPolicy {
                    max_attempts: 2,
                    base_delay: Duration::from_millis(1),
                    max_delay: Duration::from_millis(2),
                },
            },
        )
    }

    fn options() -> DiagnoseOptions {
        DiagnoseOptions {
            max_courses: None,
            sample_lectures_per_course: 5,
            include_titles: false,
        }
    }

    fn pacing() -> Pacing {
        Pacing {
            detail_delay: Duration::ZERO,
            course_concurrency: 2,
        }
    }

    fn meta() -> RunMeta {
        RunMeta {
            app_version: "9.9.9".into(),
            generated_at: "2026-01-02T03:04:05Z".into(),
            secrets: creds().secret_values(),
        }
    }

    async fn run_with(
        srv: &TestServer,
        opts: DiagnoseOptions,
        sink: &Rec,
        cancel: &CancelFlag,
    ) -> Result<UdemyDiagnostic, UdemyError> {
        run(
            &client(&srv.base()),
            &opts,
            &pacing(),
            sink,
            cancel,
            &meta(),
        )
        .await
    }

    async fn full() -> (TestServer, UdemyDiagnostic, Rec) {
        let srv = TestServer::start(udemy).await;
        let rec = Rec::default();
        let d = run_with(&srv, options(), &rec, &CancelFlag::new())
            .await
            .unwrap();
        (srv, d, rec)
    }

    fn endpoint<'a>(d: &'a UdemyDiagnostic, name: &str) -> &'a EndpointReport {
        d.endpoints.iter().find(|e| e.name == name).unwrap()
    }

    #[tokio::test]
    async fn a_full_run_reports_every_course_with_counts_and_verdicts() {
        let (_srv, d, _) = full().await;
        assert_eq!(d.account_kind, "personal");
        assert_eq!(
            (d.app_version.as_str(), d.generated_at.as_str()),
            ("9.9.9", "2026-01-02T03:04:05Z")
        );
        assert_eq!(d.status, RunStatus::Complete);
        assert_eq!(d.courses.len(), 3);
        let one = d.courses.iter().find(|c| c.id == "1001").unwrap();
        assert_eq!(one.lectures, 5);
        assert_eq!(
            (
                one.types.video,
                one.types.article,
                one.types.quiz,
                one.types.other
            ),
            (2, 1, 2, 2)
        );
        assert_eq!(one.probed, 5);
        let c = one.classification;
        assert_eq!(
            (
                c.downloadable,
                c.drm,
                c.article,
                c.external,
                c.quiz,
                c.unknown
            ),
            (1, 1, 1, 1, 2, 1)
        );
        assert_eq!(one.verdict, Verdict::Partial);
        let two = d.courses.iter().find(|c| c.id == "1002").unwrap();
        assert_eq!(
            (two.verdict, two.lectures, two.probed),
            (Verdict::Unknown, 0, 0)
        );
        assert_eq!(d.totals.courses, 3);
        assert_eq!(d.totals.lectures, 5);
        assert_eq!(d.totals.probed, 5);
        assert_eq!(d.totals.verdicts.partial, 1);
        assert_eq!(d.totals.verdicts.unknown, 2);
        assert_eq!(d.totals.classification.drm, 1);
    }

    #[tokio::test]
    async fn endpoint_statuses_make_every_assumption_explicit() {
        let (_srv, d, _) = full().await;
        let courses = endpoint(&d, "subscribedCourses");
        assert_eq!(
            (
                courses.status.as_str(),
                courses.verified,
                courses.http_status
            ),
            ("ok", true, Some(200))
        );
        let curriculum = endpoint(&d, "curriculumItems");
        assert_eq!(curriculum.status, "http_403");
        assert!(!curriculum.verified);
        assert!(curriculum.note.contains("failed"), "{}", curriculum.note);
        let detail = endpoint(&d, "lectureDetail");
        assert_eq!(detail.status, "http_404");
        assert!(detail.note.contains('4'));
        assert_eq!(d.endpoints.len(), 3);
    }

    #[tokio::test]
    async fn partial_failures_become_issues_and_drm_is_only_reported() {
        let (_srv, d, _) = full().await;
        let msgs: Vec<&str> = d.issues.iter().map(|i| i.message.as_str()).collect();
        assert!(
            msgs.iter()
                .any(|m| m.starts_with("drm_protected:") && m.contains("media_license_token")),
            "{msgs:?}"
        );
        assert!(
            msgs.iter()
                .any(|m| m.contains("curriculum") && m.contains("http_403")),
            "{msgs:?}"
        );
        assert!(
            msgs.iter()
                .any(|m| m.contains("curriculum") && m.contains("http_404")),
            "{msgs:?}"
        );
        assert!(
            msgs.iter()
                .any(|m| m.contains("lecture") && m.contains("http_404")),
            "{msgs:?}"
        );
        let drm = d
            .issues
            .iter()
            .find(|i| i.message.starts_with("drm_protected:"))
            .unwrap();
        assert_eq!(drm.severity, lumen_core::Severity::Warning);
        assert_eq!(drm.kind, lumen_core::IssueKind::VideoNotResolved);
    }

    #[tokio::test]
    async fn field_coverage_shows_which_assumed_fields_really_appeared() {
        let (_srv, d, _) = full().await;
        let items = &d.fields["curriculumItems"];
        assert_eq!(
            items["title"],
            FieldStat {
                present: 5,
                total: 5
            }
        );
        assert_eq!(items["asset.media_license_token"].present, 1);
        assert_eq!(items["asset.stream_urls"].present, 1);
        assert!(items["asset.asset_type"].total >= 4);
        let detail = &d.fields["lectureDetail"];
        assert_eq!(detail["asset"].total, 4, "the 404 has no body to inspect");
        assert!(detail["asset.course_is_drmed"].present >= 2);
        assert_eq!(d.fields["subscribedCourses"]["id"].present, 3);
    }

    #[tokio::test]
    async fn samples_are_sanitized_first_responses_of_each_endpoint() {
        let (_srv, d, _) = full().await;
        let names: Vec<&str> = d.samples.keys().map(String::as_str).collect();
        assert_eq!(
            names,
            ["curriculumItems", "lectureDetail", "subscribedCourses"]
        );
        let all = serde_json::to_string(&d.samples).unwrap();
        for leak in [
            "FAKELICENSETOKEN",
            "token=FAKE",
            EMAIL,
            "Bienvenida",
            "Curso Uno",
            "Ana",
        ] {
            assert!(!all.contains(leak), "{leak} leaked in samples");
        }
        let results = d.samples["curriculumItems"]["results"].as_array().unwrap();
        assert!(results.len() <= 3, "arrays are truncated");
        assert_eq!(results[1]["_class"], "lecture");
    }

    #[tokio::test]
    async fn titles_are_placeholders_unless_the_owner_opts_in() {
        let (_srv, d, _) = full().await;
        let one = d.courses.iter().find(|c| c.id == "1001").unwrap();
        assert_eq!(one.title, "<title len=9>");
        let srv = TestServer::start(udemy).await;
        let opts = DiagnoseOptions {
            include_titles: true,
            ..options()
        };
        let d = run_with(&srv, opts, &Rec::default(), &CancelFlag::new())
            .await
            .unwrap();
        assert_eq!(
            d.courses.iter().find(|c| c.id == "1001").unwrap().title,
            "Curso Uno"
        );
    }

    #[tokio::test]
    async fn progress_events_follow_the_steps_and_finish_last() {
        let (_srv, _, rec) = full().await;
        let ev = rec.events.lock().unwrap();
        let steps: Vec<Step> = ev.iter().map(|e| e.step).collect();
        assert_eq!(steps.first(), Some(&Step::Session));
        assert_eq!(steps.last(), Some(&Step::Finish));
        assert!(
            steps.contains(&Step::Courses)
                && steps.contains(&Step::Curriculum)
                && steps.contains(&Step::Lectures)
        );
        let courses = ev.iter().rev().find(|e| e.step == Step::Courses).unwrap();
        assert_eq!(courses.total, 3);
        let last = ev.last().unwrap();
        assert_eq!(last.current, last.total);
        let json = serde_json::to_value(courses).unwrap();
        assert_eq!(json["step"], "courses");
        assert!(json["message"].is_string());
    }

    #[tokio::test]
    async fn limits_bound_the_work() {
        let srv = TestServer::start(udemy).await;
        let opts = DiagnoseOptions {
            max_courses: Some(1),
            sample_lectures_per_course: 1,
            ..options()
        };
        let d = run_with(&srv, opts, &Rec::default(), &CancelFlag::new())
            .await
            .unwrap();
        assert_eq!(d.courses.len(), 1);
        assert_eq!(d.courses[0].probed, 1);
        let details = srv
            .requests()
            .iter()
            .filter(|r| r.path_and_query.contains("/lectures/"))
            .count();
        assert_eq!(details, 1);
        assert_eq!(
            d.courses[0].classification.downloadable, 1,
            "the first video lecture"
        );
    }

    #[tokio::test]
    async fn cancel_stops_the_run_and_marks_it() {
        let srv = TestServer::start(udemy).await;
        let cancel = CancelFlag::new();
        let rec = Rec {
            cancel_at: Some((Step::Curriculum, cancel.clone())),
            ..Default::default()
        };
        let d = run_with(&srv, options(), &rec, &cancel).await.unwrap();
        assert_eq!(d.status, RunStatus::Cancelled);
        let details = srv
            .requests()
            .iter()
            .filter(|r| r.path_and_query.contains("/lectures/"))
            .count();
        assert!(details < 5, "{details}");
        assert_eq!(
            rec.events.lock().unwrap().last().unwrap().step,
            Step::Finish
        );
    }

    #[tokio::test]
    async fn an_invalid_session_or_a_bot_challenge_stops_before_anything_else() {
        let srv = TestServer::start(|_, _| Reply::status(401)).await;
        let err = run_with(&srv, options(), &Rec::default(), &CancelFlag::new())
            .await
            .unwrap_err();
        assert_eq!(err, UdemyError::Unauthorized);
        assert_eq!(srv.count(), 1);
        let srv = TestServer::start(|_, _| challenge()).await;
        let err = run_with(&srv, options(), &Rec::default(), &CancelFlag::new())
            .await
            .unwrap_err();
        assert_eq!(err, UdemyError::BotChallenge);
    }

    #[tokio::test]
    async fn a_session_that_expires_mid_run_aborts_with_what_was_learned() {
        let srv = TestServer::start(|req: &Recorded, n| {
            if req.path_and_query.contains("/lectures/") {
                Reply::status(401)
            } else {
                udemy(req, n)
            }
        })
        .await;
        let d = run_with(&srv, options(), &Rec::default(), &CancelFlag::new())
            .await
            .unwrap();
        assert_eq!(d.status, RunStatus::Aborted);
        assert!(d
            .issues
            .iter()
            .any(|i| i.severity == lumen_core::Severity::Error
                && i.message.contains("session expired")));
        assert_eq!(endpoint(&d, "lectureDetail").status, "http_401");
        assert!(
            d.courses.iter().any(|c| c.id == "1001"),
            "partial results are kept"
        );
    }

    #[tokio::test]
    async fn the_written_file_is_private_pretty_json_without_any_secret() {
        let (_srv, d, _) = full().await;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("udemy-diagnostico.json");
        d.write_to(&path, &meta().secrets).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        for leak in [TOKEN, "cid12345", EMAIL, "FAKELICENSETOKEN"] {
            assert!(!text.contains(leak), "{leak} leaked into the file");
        }
        assert!(
            text.contains("\n  \"accountKind\": \"personal\""),
            "pretty JSON"
        );
        let back: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(back["totals"]["courses"], 3);
        assert_eq!(back["status"], "complete");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(!tmp.path().join("udemy-diagnostico.json.tmp").exists());
    }

    #[test]
    fn the_sample_covers_video_article_resource_and_last_lecture() {
        let page = crate::parse::parse_curriculum_page(
            &serde_json::from_str::<Value>(include_str!(
                "../tests/fixtures/assumed_curriculum_page1.json"
            ))
            .unwrap(),
        )
        .unwrap();
        let lectures: Vec<&crate::parse::CurriculumItem> = page
            .items
            .iter()
            .filter(|i| i.kind == crate::parse::ItemKind::Lecture)
            .collect();
        let ids = |n: usize| -> Vec<&str> {
            pick_sample(&lectures, n)
                .into_iter()
                .map(|i| lectures[i].id.as_str())
                .collect()
        };
        assert_eq!(ids(5), ["11", "12", "22", "21"]);
        assert_eq!(ids(3), ["11", "12", "22"]);
        assert_eq!(ids(0), Vec::<&str>::new());
        assert_eq!(ids(1), ["11"]);
        assert_eq!(ids(2), ["11", "12"]);
        assert_eq!(pick_sample(&lectures, 99).len(), lectures.len());
        assert!(pick_sample(&[], 5).is_empty());
    }

    #[test]
    fn timestamps_are_iso_8601_utc() {
        assert_eq!(iso8601_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601_utc(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(iso8601_utc(951_827_696), "2000-02-29T12:34:56Z");
        assert_eq!(iso8601_utc(4_102_444_799), "2099-12-31T23:59:59Z");
    }
}
