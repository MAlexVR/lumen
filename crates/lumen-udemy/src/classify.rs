//! Per-lecture classification and the per-course verdict. HARD RULE: DRM is
//! only detected and reported, never requested or handled. HYPOTHESES
//! (unverified until a real diagnostic run): a non-empty `media_license_token`,
//! `course_is_drmed: true`, or DASH as the only source mean DRM; HLS without a
//! token and progressive files are downloadable.

use serde::Serialize;

use crate::parse::{Asset, ItemKind, MediaUrl};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LectureClass {
    Downloadable,
    Drm,
    Article,
    External,
    Quiz,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    pub class: LectureClass,
    pub reason: Option<String>,
}

fn of(class: LectureClass, reason: impl Into<String>) -> Classification {
    Classification {
        class,
        reason: Some(reason.into()),
    }
}

/// URL path without query or fragment, lowercase.
fn path_of(url: &str) -> String {
    url.split(['?', '#'])
        .next()
        .unwrap_or_default()
        .to_lowercase()
}

fn is_dash(m: &MediaUrl) -> bool {
    m.mime
        .as_deref()
        .is_some_and(|t| t.to_lowercase().contains("dash"))
        || path_of(&m.url).ends_with(".mpd")
}

fn is_hls(m: &MediaUrl) -> bool {
    m.mime
        .as_deref()
        .is_some_and(|t| t.to_lowercase().contains("mpegurl"))
        || path_of(&m.url).ends_with(".m3u8")
}

fn is_progressive(m: &MediaUrl) -> bool {
    if is_dash(m) || is_hls(m) {
        return false;
    }
    let by_mime = m
        .mime
        .as_deref()
        .is_some_and(|t| t.starts_with("video/") || t.starts_with("audio/"));
    let path = path_of(&m.url);
    by_mime
        || [".mp4", ".webm", ".m4a", ".mp3", ".mkv", ".mov"]
            .iter()
            .any(|e| path.ends_with(e))
}

fn media(a: &Asset) -> impl Iterator<Item = &MediaUrl> {
    a.streams.iter().chain(&a.sources).chain(&a.downloads)
}

fn classify_asset(a: &Asset) -> Classification {
    if a.has_license_token {
        return of(LectureClass::Drm, "media_license_token present");
    }
    if a.course_is_drmed == Some(true) {
        return of(LectureClass::Drm, "course_is_drmed is true");
    }
    let all: Vec<&MediaUrl> = media(a).collect();
    if !all.is_empty() && all.iter().all(|m| is_dash(m)) {
        return of(LectureClass::Drm, "only DASH sources are exposed");
    }
    if a.is_external || a.external_url.is_some() {
        return of(LectureClass::External, "external link");
    }
    let is_article = a
        .asset_type
        .as_deref()
        .is_some_and(|t| t.eq_ignore_ascii_case("article"));
    if is_article || (a.body.is_some() && all.is_empty()) {
        return of(LectureClass::Article, "html body");
    }
    if all.iter().any(|m| is_progressive(m)) {
        return of(LectureClass::Downloadable, "progressive stream");
    }
    if all.iter().any(|m| is_hls(m)) {
        return of(LectureClass::Downloadable, "HLS source without a license");
    }
    if !all.is_empty() {
        return of(LectureClass::Downloadable, "downloadable file");
    }
    of(
        LectureClass::Unknown,
        format!(
            "asset type {} exposes no media",
            a.asset_type.as_deref().unwrap_or("unknown")
        ),
    )
}

pub fn classify_item(
    kind: &ItemKind,
    asset: Option<&Asset>,
    supplementary: &[Asset],
) -> Classification {
    match kind {
        ItemKind::Quiz | ItemKind::Practice => of(LectureClass::Quiz, "not downloadable"),
        ItemKind::Chapter | ItemKind::Other(_) => of(LectureClass::Unknown, "not a lecture"),
        ItemKind::Lecture => match asset {
            Some(a) => classify_asset(a),
            None if supplementary.iter().any(|s| media(s).next().is_some()) => {
                of(LectureClass::Downloadable, "resources only")
            }
            None => of(LectureClass::Unknown, "no asset in the response"),
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassCounts {
    pub downloadable: u32,
    pub drm: u32,
    pub article: u32,
    pub external: u32,
    pub quiz: u32,
    pub unknown: u32,
}

impl ClassCounts {
    pub fn add(&mut self, class: LectureClass) {
        let slot = match class {
            LectureClass::Downloadable => &mut self.downloadable,
            LectureClass::Drm => &mut self.drm,
            LectureClass::Article => &mut self.article,
            LectureClass::External => &mut self.external,
            LectureClass::Quiz => &mut self.quiz,
            LectureClass::Unknown => &mut self.unknown,
        };
        *slot += 1;
    }

    pub fn merge(&mut self, other: &Self) {
        self.downloadable += other.downloadable;
        self.drm += other.drm;
        self.article += other.article;
        self.external += other.external;
        self.quiz += other.quiz;
        self.unknown += other.unknown;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Downloadable,
    Partial,
    None,
    Unknown,
}

/// What can be downloaded from a course, judged on the lectures probed.
/// Quizzes say nothing; external links are neutral.
pub fn verdict(c: &ClassCounts) -> Verdict {
    let playable = c.downloadable + c.article;
    if playable == 0 {
        return if c.drm > 0 {
            Verdict::None
        } else {
            Verdict::Unknown
        };
    }
    if c.drm == 0 && c.unknown == 0 {
        Verdict::Downloadable
    } else {
        Verdict::Partial
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{parse_curriculum_page, parse_lecture_detail, Asset, ItemKind};
    use serde_json::{json, Value};

    fn asset(v: Value) -> Asset {
        let page = parse_curriculum_page(
            &json!({"results": [{"_class": "lecture", "id": 1, "asset": v}]}),
        )
        .unwrap();
        page.items[0].asset.clone().unwrap()
    }

    fn lecture(a: Option<&Asset>, supp: &[Asset]) -> Classification {
        classify_item(&ItemKind::Lecture, a, supp)
    }

    #[test]
    fn a_progressive_stream_is_downloadable() {
        let a = asset(
            json!({"asset_type": "Video", "stream_urls": {"Video": [{"type": "video/mp4", "file": "https://a.test/v.mp4"}]}}),
        );
        let c = lecture(Some(&a), &[]);
        assert_eq!(c.class, LectureClass::Downloadable);
        assert!(c.reason.unwrap().contains("progressive"));
    }

    #[test]
    fn hls_without_a_license_is_downloadable_but_dash_alone_is_drm() {
        let hls = asset(
            json!({"asset_type": "Video", "media_sources": [{"type": "application/x-mpegURL", "src": "https://a.test/i.m3u8"}]}),
        );
        assert_eq!(lecture(Some(&hls), &[]).class, LectureClass::Downloadable);
        let dash = asset(
            json!({"asset_type": "Video", "media_sources": [{"type": "application/dash+xml", "src": "https://a.test/m.mpd"}]}),
        );
        let c = lecture(Some(&dash), &[]);
        assert_eq!(c.class, LectureClass::Drm);
        assert!(c.reason.unwrap().contains("DASH"));
        let mpd_by_extension = asset(
            json!({"asset_type": "Video", "media_sources": [{"src": "https://a.test/m.mpd?x=1"}]}),
        );
        assert_eq!(
            lecture(Some(&mpd_by_extension), &[]).class,
            LectureClass::Drm
        );
    }

    #[test]
    fn a_license_token_or_a_drm_flag_always_wins() {
        let token = asset(
            json!({"asset_type": "Video", "media_license_token": "abcdefabcdef",
            "stream_urls": {"Video": [{"type": "video/mp4", "file": "https://a.test/v.mp4"}]}}),
        );
        assert_eq!(lecture(Some(&token), &[]).class, LectureClass::Drm);
        let flag = asset(json!({"asset_type": "Video", "course_is_drmed": true,
            "media_sources": [{"type": "application/x-mpegURL", "src": "https://a.test/i.m3u8"}]}));
        let c = lecture(Some(&flag), &[]);
        assert_eq!(c.class, LectureClass::Drm);
        assert!(c.reason.unwrap().contains("course_is_drmed"));
        let empty_token = asset(json!({"asset_type": "Video", "media_license_token": "",
            "stream_urls": {"Video": [{"type": "video/mp4", "file": "https://a.test/v.mp4"}]}}));
        assert_eq!(
            lecture(Some(&empty_token), &[]).class,
            LectureClass::Downloadable
        );
    }

    #[test]
    fn articles_external_links_and_quizzes_have_their_own_classes() {
        let article = asset(json!({"asset_type": "Article", "body": "<p>x</p>"}));
        assert_eq!(lecture(Some(&article), &[]).class, LectureClass::Article);
        let body_only = asset(json!({"body": "<p>x</p>"}));
        assert_eq!(lecture(Some(&body_only), &[]).class, LectureClass::Article);
        let ext = asset(
            json!({"asset_type": "ExternalLink", "is_external": true, "external_url": "https://e.test"}),
        );
        assert_eq!(lecture(Some(&ext), &[]).class, LectureClass::External);
        let ext_url = asset(json!({"external_url": "https://e.test"}));
        assert_eq!(lecture(Some(&ext_url), &[]).class, LectureClass::External);
        assert_eq!(
            classify_item(&ItemKind::Quiz, None, &[]).class,
            LectureClass::Quiz
        );
        assert_eq!(
            classify_item(&ItemKind::Practice, None, &[]).class,
            LectureClass::Quiz
        );
    }

    #[test]
    fn downloadable_files_and_resources_count() {
        let file = asset(
            json!({"asset_type": "File", "download_urls": {"File": [{"file": "https://a.test/f.pdf"}]}}),
        );
        assert_eq!(lecture(Some(&file), &[]).class, LectureClass::Downloadable);
        let resources_only = lecture(None, std::slice::from_ref(&file));
        assert_eq!(resources_only.class, LectureClass::Downloadable);
        assert!(resources_only.reason.unwrap().contains("resources only"));
    }

    #[test]
    fn anything_else_is_unknown_with_the_reason() {
        let c = lecture(None, &[]);
        assert_eq!(c.class, LectureClass::Unknown);
        assert!(c.reason.unwrap().contains("no asset"));
        let odd = asset(json!({"asset_type": "Presentation"}));
        let c = lecture(Some(&odd), &[]);
        assert_eq!(c.class, LectureClass::Unknown);
        assert!(c.reason.unwrap().contains("Presentation"));
        let c = classify_item(&ItemKind::Chapter, None, &[]);
        assert_eq!(c.class, LectureClass::Unknown);
        let from_detail = parse_lecture_detail(&json!({"asset": {"asset_type": "Video"}}))
            .unwrap()
            .asset
            .unwrap();
        assert_eq!(
            lecture(Some(&from_detail), &[]).class,
            LectureClass::Unknown
        );
    }

    #[test]
    fn counts_and_course_verdicts() {
        let mut c = ClassCounts::default();
        assert_eq!(verdict(&c), Verdict::Unknown);
        c.add(LectureClass::Quiz);
        assert_eq!(verdict(&c), Verdict::Unknown, "quizzes alone say nothing");
        c.add(LectureClass::Downloadable);
        c.add(LectureClass::Article);
        c.add(LectureClass::External);
        assert_eq!(verdict(&c), Verdict::Downloadable);
        c.add(LectureClass::Drm);
        assert_eq!(verdict(&c), Verdict::Partial);
        let mut all_drm = ClassCounts::default();
        all_drm.add(LectureClass::Drm);
        all_drm.add(LectureClass::Drm);
        assert_eq!(verdict(&all_drm), Verdict::None);
        let mut unk = ClassCounts::default();
        unk.add(LectureClass::Unknown);
        assert_eq!(verdict(&unk), Verdict::Unknown);
        let mut mix = ClassCounts::default();
        mix.add(LectureClass::Downloadable);
        mix.add(LectureClass::Unknown);
        assert_eq!(verdict(&mix), Verdict::Partial);
        let json = serde_json::to_value(c).unwrap();
        assert_eq!(json["downloadable"], 1);
        assert_eq!(json["drm"], 1);
        assert_eq!(serde_json::to_value(Verdict::Partial).unwrap(), "partial");
        assert_eq!(serde_json::to_value(LectureClass::Drm).unwrap(), "drm");
    }
}
