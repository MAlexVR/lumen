//! Pure parsers from Skool HTML pages (`__NEXT_DATA__`) to typed values.

use std::sync::OnceLock;

use lumen_core::PlatformGroup;
use regex::Regex;
use serde_json::Value;

use crate::error::SkoolError;
use crate::richtext::strip_emoji;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CourseInfo {
    pub id: String,
    /// URL slug of the course (`/classroom/<short_name>`).
    pub short_name: String,
    pub title: String,
    pub num_modules: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedCourse {
    pub title: String,
    pub reason: SkipReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// `privacy == 2`: hidden course, skipped silently.
    Private,
    NoAccess,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CourseListing {
    pub courses: Vec<CourseInfo>,
    pub skipped: Vec<SkippedCourse>,
}

/// One lesson as listed in the course tree (only partially populated).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleEntry {
    pub id: String,
    pub section: String,
    pub title: String,
    pub video_link: String,
    pub video_id: String,
    pub desc: String,
    pub resources: String,
    pub has_access: bool,
}

impl ModuleEntry {
    /// Hosted by Skool (Mux): has a video id but no external link.
    pub fn is_native(&self) -> bool {
        !self.video_id.is_empty() && self.video_link.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CourseTree {
    pub title: String,
    pub modules: Vec<ModuleEntry>,
}

/// Mux playback data from `pageProps.video`. Debug hides the token.
#[derive(Clone, PartialEq, Eq)]
pub struct NativeVideo {
    pub playback_id: String,
    pub token: String,
    pub expire: Option<u64>,
}

impl std::fmt::Debug for NativeVideo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeVideo")
            .field("playback_id", &self.playback_id)
            .field("token", &"<redacted>")
            .field("expire", &self.expire)
            .finish()
    }
}

impl NativeVideo {
    /// Stream URL for yt-dlp. Contains the token: never log it.
    pub fn stream_url(&self) -> String {
        format!(
            "https://stream.mux.com/{}.m3u8?token={}",
            self.playback_id, self.token
        )
    }
}

/// Fully populated data of one module page (`?md=<id>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleDetail {
    pub desc: String,
    pub resources: String,
    pub video_link: String,
    pub video_id: String,
    pub native: Option<NativeVideo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceLink {
    pub title: String,
    pub link: String,
}

/// Returns `props.pageProps` from a Skool HTML page.
pub fn extract_page_props(html: &str) -> Result<Value, SkoolError> {
    let caps = next_data_re()
        .captures(html)
        .ok_or(SkoolError::SessionBlocked)?;
    let mut parsed: Value =
        serde_json::from_str(&caps[1]).map_err(|e| SkoolError::Json(e.to_string()))?;
    match parsed.pointer_mut("/props/pageProps") {
        Some(pp) => Ok(pp.take()),
        None => Err(SkoolError::Shape("__NEXT_DATA__ has no pageProps".into())),
    }
}

fn next_data_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // The pattern is a constant; compilation cannot fail.
        Regex::new(r#"(?s)<script id="__NEXT_DATA__"[^>]*>(.*?)</script>"#)
            .unwrap_or_else(|_| unreachable!("static regex"))
    })
}

/// String value of `key`: strings as-is, numbers formatted, anything else empty.
fn text(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// Groups from `api2.skool.com/self/groups`.
pub fn parse_groups(body: &Value) -> Vec<PlatformGroup> {
    let Some(groups) = body.get("groups").and_then(Value::as_array) else {
        return Vec::new();
    };
    groups
        .iter()
        .filter_map(|g| {
            let slug = text(g, "name");
            if slug.is_empty() {
                return None;
            }
            let meta = g.get("metadata").unwrap_or(&Value::Null);
            let display = strip_emoji(&text(meta, "display_name"));
            let cover = text(meta, "cover_small_url");
            Some(PlatformGroup {
                name: if display.is_empty() {
                    slug.clone()
                } else {
                    display
                },
                slug,
                cover_url: (!cover.is_empty()).then_some(cover),
                num_courses: meta
                    .get("num_courses")
                    .and_then(Value::as_u64)
                    .and_then(|n| u32::try_from(n).ok())
                    .unwrap_or(0),
            })
        })
        .collect()
}

pub fn parse_courses(page_props: &Value) -> CourseListing {
    let mut out = CourseListing::default();
    let Some(all) = page_props.get("allCourses").and_then(Value::as_array) else {
        return out;
    };
    for c in all {
        let Some(meta) = c.get("metadata") else {
            continue;
        };
        let short_name = text(c, "name");
        let title = match text(meta, "title") {
            t if t.is_empty() => short_name.clone(),
            t => t,
        };
        if meta.get("privacy").and_then(Value::as_i64) == Some(2) {
            out.skipped.push(SkippedCourse {
                title,
                reason: SkipReason::Private,
            });
        } else if flag(meta.get("hasAccess")) != Some(true) {
            out.skipped.push(SkippedCourse {
                title,
                reason: SkipReason::NoAccess,
            });
        } else {
            out.courses.push(CourseInfo {
                id: text(c, "id"),
                short_name,
                title,
                num_modules: meta
                    .get("numModules")
                    .and_then(Value::as_u64)
                    .and_then(|n| u32::try_from(n).ok())
                    .unwrap_or(0),
            });
        }
    }
    out
}

pub fn parse_course_tree(page_props: &Value) -> Result<CourseTree, SkoolError> {
    let course = page_props
        .get("course")
        .ok_or_else(|| SkoolError::Shape("page has no course".into()))?;
    let title = match course.pointer("/course/metadata").map(|m| text(m, "title")) {
        Some(t) if !t.is_empty() => t,
        _ => "Course".to_string(),
    };
    let mut modules = Vec::new();
    for node in children(course) {
        let Some(unit) = node.get("course") else {
            continue;
        };
        let meta = unit.get("metadata").unwrap_or(&Value::Null);
        match text(unit, "unitType").as_str() {
            "module" => modules.push(module_entry(&title, unit, meta)),
            "set" => {
                let section = match text(meta, "title") {
                    t if t.is_empty() => "Section".to_string(),
                    t => t,
                };
                for child in children(node) {
                    let Some(m) = child.get("course") else {
                        continue;
                    };
                    if text(m, "unitType") == "module" {
                        let mm = m.get("metadata").unwrap_or(&Value::Null);
                        modules.push(module_entry(&section, m, mm));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(CourseTree { title, modules })
}

/// Skool sends some flags as booleans and others as 0/1 integers.
fn flag(value: Option<&Value>) -> Option<bool> {
    let value = value?;
    value.as_bool().or_else(|| value.as_i64().map(|n| n != 0))
}

fn children(node: &Value) -> &[Value] {
    node.get("children")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn module_entry(section: &str, unit: &Value, meta: &Value) -> ModuleEntry {
    ModuleEntry {
        id: text(unit, "id"),
        section: section.to_string(),
        title: text(meta, "title"),
        video_link: text(meta, "videoLink"),
        video_id: text(meta, "videoId"),
        desc: text(meta, "desc"),
        resources: text(meta, "resources"),
        has_access: flag(meta.get("hasAccess")).unwrap_or(true),
    }
}

/// Finds a module's `course` node by id, searching nested sets.
fn find_module<'a>(nodes: &'a [Value], id: &str) -> Option<&'a Value> {
    for node in nodes {
        if let Some(unit) = node.get("course") {
            if text(unit, "id") == id {
                return Some(unit);
            }
        }
        if let Some(found) = find_module(children(node), id) {
            return Some(found);
        }
    }
    None
}

pub fn parse_module_page(page_props: &Value, module_id: &str) -> Result<ModuleDetail, SkoolError> {
    let course = page_props
        .get("course")
        .ok_or_else(|| SkoolError::Shape("module page has no course data".into()))?;
    let unit = find_module(children(course), module_id)
        .ok_or_else(|| SkoolError::Shape(format!("module {module_id} not found in its page")))?;
    let meta = unit.get("metadata").unwrap_or(&Value::Null);
    let native = page_props.get("video").and_then(|v| {
        let playback_id = text(v, "playbackId");
        let token = text(v, "playbackToken");
        (!playback_id.is_empty() && !token.is_empty()).then(|| NativeVideo {
            playback_id,
            token,
            expire: v.get("expire").and_then(Value::as_u64),
        })
    });
    Ok(ModuleDetail {
        desc: text(meta, "desc"),
        resources: text(meta, "resources"),
        video_link: text(meta, "videoLink"),
        video_id: text(meta, "videoId"),
        native,
    })
}

/// Parses the `resources` metadata (JSON string of `[{title, link}]`).
/// Empty, `[]` and `false` mean no resources.
pub fn parse_resources(raw: &str) -> Result<Vec<ResourceLink>, SkoolError> {
    let raw = raw.trim();
    if raw.is_empty() || raw == "false" {
        return Ok(Vec::new());
    }
    let items: Vec<Value> =
        serde_json::from_str(raw).map_err(|e| SkoolError::Json(format!("resources: {e}")))?;
    Ok(items
        .iter()
        .filter_map(|r| {
            let link = text(r, "link");
            (!link.is_empty()).then(|| ResourceLink {
                title: text(r, "title"),
                link,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLASSROOM: &str = include_str!("../tests/fixtures/classroom.html");
    const COURSE: &str = include_str!("../tests/fixtures/course.html");
    const MODULE: &str = include_str!("../tests/fixtures/module.html");
    const GROUPS: &str = include_str!("../tests/fixtures/groups.json");

    fn props(html: &str) -> Value {
        extract_page_props(html).unwrap()
    }

    #[test]
    fn extracts_page_props_from_next_data() {
        let pp = props(CLASSROOM);
        assert_eq!(pp["allCourses"].as_array().map(Vec::len), Some(4));
    }

    #[test]
    fn missing_next_data_means_session_blocked() {
        assert_eq!(
            extract_page_props("<html>login</html>"),
            Err(SkoolError::SessionBlocked)
        );
    }

    #[test]
    fn invalid_json_and_missing_page_props_are_distinct_errors() {
        let bad = r#"<script id="__NEXT_DATA__" type="application/json">{nope</script>"#;
        assert!(matches!(extract_page_props(bad), Err(SkoolError::Json(_))));
        let none = r#"<script id="__NEXT_DATA__" type="application/json">{"props":{}}</script>"#;
        assert!(matches!(
            extract_page_props(none),
            Err(SkoolError::Shape(_))
        ));
    }

    #[test]
    fn groups_use_slug_display_name_without_emoji_and_optional_cover() {
        let body: Value = serde_json::from_str(GROUPS).unwrap();
        let g = parse_groups(&body);
        assert_eq!(g.len(), 3);
        assert_eq!(g[0].slug, "growth-lab");
        assert_eq!(g[0].name, "Growth Lab");
        assert_eq!(
            g[0].cover_url.as_deref(),
            Some("https://assets.skool.com/c/growth.jpg")
        );
        assert_eq!(g[0].num_courses, 3);
        assert_eq!(g[1].name, "no-meta");
        assert_eq!(g[1].cover_url, None);
        assert_eq!(g[2].cover_url, None);
    }

    #[test]
    fn groups_of_unexpected_shape_are_empty() {
        assert!(parse_groups(&serde_json::json!({"x": 1})).is_empty());
    }

    #[test]
    fn courses_skip_private_and_inaccessible_and_fall_back_to_the_short_name() {
        let l = parse_courses(&props(CLASSROOM));
        let names: Vec<_> = l.courses.iter().map(|c| c.short_name.as_str()).collect();
        assert_eq!(names, ["intro-course", "no-title"]);
        assert_eq!(l.courses[0].title, "Intro Course");
        assert_eq!(l.courses[0].num_modules, 3);
        assert_eq!(l.courses[0].id, "c1");
        assert_eq!(l.courses[1].title, "no-title");
        assert_eq!(l.skipped.len(), 2);
        assert_eq!(l.skipped[0].reason, SkipReason::Private);
        assert_eq!(l.skipped[1].reason, SkipReason::NoAccess);
        assert_eq!(l.skipped[1].title, "Locked");
    }

    #[test]
    fn access_flags_accept_booleans_and_integers() {
        let page = serde_json::json!({"allCourses": [
            {"id": "a", "name": "bool-yes", "metadata": {"title": "A", "hasAccess": true, "privacy": 0}},
            {"id": "b", "name": "int-yes", "metadata": {"title": "B", "hasAccess": 1, "privacy": 0}},
            {"id": "c", "name": "int-no", "metadata": {"title": "C", "hasAccess": 0, "privacy": 0}},
            {"id": "d", "name": "missing", "metadata": {"title": "D", "privacy": 0}}
        ]});
        let l = parse_courses(&page);
        let names: Vec<_> = l.courses.iter().map(|c| c.short_name.as_str()).collect();
        assert_eq!(names, ["bool-yes", "int-yes"]);
        assert_eq!(l.skipped.len(), 2);
        assert!(l.skipped.iter().all(|s| s.reason == SkipReason::NoAccess));
    }

    #[test]
    fn module_access_flag_accepts_integers() {
        let entry = |flag: serde_json::Value| {
            let meta = serde_json::json!({"title": "T", "hasAccess": flag});
            module_entry("S", &serde_json::json!({"id": "m"}), &meta)
        };
        assert!(entry(serde_json::json!(1)).has_access);
        assert!(!entry(serde_json::json!(0)).has_access);
        assert!(!entry(serde_json::json!(false)).has_access);
    }

    #[test]
    fn tree_lists_modules_in_order_with_sections() {
        let t = parse_course_tree(&props(COURSE)).unwrap();
        assert_eq!(t.title, "Intro Course");
        let rows: Vec<_> = t
            .modules
            .iter()
            .map(|m| (m.id.as_str(), m.section.as_str(), m.title.as_str()))
            .collect();
        assert_eq!(
            rows,
            [
                ("m1", "Getting Started", "Welcome"),
                ("m2", "Getting Started", "External"),
                ("m3", "Intro Course", "Standalone"),
                ("m4", "Section", "Untitled set lesson"),
            ]
        );
    }

    #[test]
    fn native_means_video_id_without_link() {
        let t = parse_course_tree(&props(COURSE)).unwrap();
        assert!(t.modules[0].is_native());
        assert!(!t.modules[1].is_native());
        assert_eq!(
            t.modules[1].video_link,
            "https://www.youtube.com/watch?v=abc"
        );
        assert!(!t.modules[2].is_native());
        assert!(t.modules[0].has_access);
    }

    #[test]
    fn tree_without_course_is_a_shape_error() {
        assert!(matches!(
            parse_course_tree(&serde_json::json!({})),
            Err(SkoolError::Shape(_))
        ));
    }

    #[test]
    fn module_page_has_description_resources_and_native_video() {
        let d = parse_module_page(&props(MODULE), "m1").unwrap();
        assert!(d.desc.starts_with("[v2]"));
        assert!(d.resources.contains("assets.skool.com"));
        let v = d.native.unwrap();
        assert_eq!(
            v.stream_url(),
            "https://stream.mux.com/PB123.m3u8?token=TOK456"
        );
        assert_eq!(v.expire, Some(1_893_456_000));
        assert!(!format!("{v:?}").contains("TOK456"));
    }

    #[test]
    fn module_page_for_unknown_module_is_an_error() {
        assert!(matches!(
            parse_module_page(&props(MODULE), "nope"),
            Err(SkoolError::Shape(_))
        ));
    }

    #[test]
    fn module_without_video_object_has_no_native_stream() {
        let mut pp = props(MODULE);
        pp.as_object_mut().unwrap().remove("video");
        assert_eq!(parse_module_page(&pp, "m1").unwrap().native, None);
        let mut pp = props(MODULE);
        pp["video"]["playbackToken"] = Value::String(String::new());
        assert_eq!(parse_module_page(&pp, "m1").unwrap().native, None);
    }

    #[test]
    fn resources_parse_and_empty_placeholders_are_empty() {
        let r = parse_resources(r#"[{"title":"A","link":"https://x.test/a.pdf"}]"#).unwrap();
        assert_eq!(
            r,
            [ResourceLink {
                title: "A".into(),
                link: "https://x.test/a.pdf".into()
            }]
        );
        for empty in ["", "[]", "false", "  "] {
            assert!(parse_resources(empty).unwrap().is_empty(), "{empty:?}");
        }
        assert!(parse_resources("{broken").is_err());
    }

    #[test]
    fn resources_without_a_link_are_dropped() {
        let r = parse_resources(
            r#"[{"title":"A","link":""},{"title":"B"},{"link":"https://x.test/c"}]"#,
        )
        .unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].title, "");
        assert_eq!(r[0].link, "https://x.test/c");
    }
}
