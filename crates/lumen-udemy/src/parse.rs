//! Tolerant parsers over Udemy's JSON. HYPOTHESIS (unverified until a real
//! diagnostic run): the field names and shapes below. Every field is optional
//! and a wrong type degrades to `None`; only a response without the expected
//! envelope (`results`) is an error, and one broken item never fails a page.

use serde::Serialize;
use serde_json::Value;

use crate::error::UdemyError;

/// Recursion limit when hunting for URLs inside unknown shapes.
const MAX_URL_DEPTH: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CourseSummary {
    pub id: String,
    pub title: Option<String>,
    pub cover_url: Option<String>,
    pub lectures: Option<u32>,
    pub instructors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoursesPage {
    pub courses: Vec<CourseSummary>,
    pub next: Option<String>,
    pub count: Option<u64>,
    /// Entries that could not be read (no id, not an object).
    pub skipped: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemKind {
    /// A section of the course.
    Chapter,
    Lecture,
    Quiz,
    Practice,
    Other(String),
}

/// A URL found inside an asset, with the mime type and label when given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaUrl {
    pub url: String,
    pub mime: Option<String>,
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caption {
    pub locale: Option<String>,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Asset {
    pub id: Option<String>,
    pub asset_type: Option<String>,
    pub title: Option<String>,
    pub filename: Option<String>,
    pub status: Option<String>,
    pub time_estimation: Option<u64>,
    pub is_external: bool,
    pub external_url: Option<String>,
    pub course_is_drmed: Option<bool>,
    /// A non-empty `media_license_token` was present (the value is dropped).
    pub has_license_token: bool,
    pub streams: Vec<MediaUrl>,
    pub downloads: Vec<MediaUrl>,
    pub sources: Vec<MediaUrl>,
    pub captions: Vec<Caption>,
    /// HTML body of an article.
    pub body: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurriculumItem {
    pub kind: ItemKind,
    pub id: String,
    pub title: Option<String>,
    pub index: Option<u64>,
    pub published: Option<bool>,
    pub asset: Option<Asset>,
    pub supplementary: Vec<Asset>,
    pub has_description: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurriculumPage {
    pub items: Vec<CurriculumItem>,
    pub next: Option<String>,
    pub skipped: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LectureDetail {
    pub asset: Option<Asset>,
    pub description: Option<String>,
    pub download_url: Option<String>,
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// A number or a numeric string, as an id.
fn id_of(v: &Value, key: &str) -> Option<String> {
    match v.get(key)? {
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => None,
    }
}

fn number(v: &Value, key: &str) -> Option<u64> {
    match v.get(key)? {
        Value::Number(n) => n.as_u64().or_else(|| n.as_f64().map(|f| f.max(0.0) as u64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn boolean(v: &Value, key: &str) -> Option<bool> {
    v.get(key).and_then(Value::as_bool)
}

fn envelope(v: &Value) -> Result<(&Vec<Value>, Option<String>), UdemyError> {
    let results = v
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| UdemyError::Shape("the response has no `results` list".into()))?;
    Ok((results, text(v, "next")))
}

pub fn parse_courses_page(v: &Value) -> Result<CoursesPage, UdemyError> {
    let (results, next) = envelope(v)?;
    let mut courses = Vec::new();
    let mut skipped = 0;
    for item in results {
        let Some(id) = id_of(item, "id") else {
            skipped += 1;
            continue;
        };
        courses.push(CourseSummary {
            id,
            title: text(item, "title"),
            cover_url: text(item, "image_480x270"),
            lectures: number(item, "num_published_lectures").and_then(|n| u32::try_from(n).ok()),
            instructors: item
                .get("visible_instructors")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|i| text(i, "display_name")).collect())
                .unwrap_or_default(),
        });
    }
    Ok(CoursesPage {
        courses,
        next,
        count: number(v, "count"),
        skipped,
    })
}

/// Every URL-looking value inside `v`: bare strings, or objects with
/// `file`/`src`/`url`/`download_url`, searched through unknown nesting.
fn collect_urls(v: Option<&Value>) -> Vec<MediaUrl> {
    fn walk(v: &Value, depth: usize, out: &mut Vec<MediaUrl>) {
        if depth > MAX_URL_DEPTH {
            return;
        }
        match v {
            Value::String(s) if s.starts_with("http") => out.push(MediaUrl {
                url: s.clone(),
                mime: None,
                label: None,
            }),
            Value::Array(a) => a.iter().for_each(|x| walk(x, depth + 1, out)),
            Value::Object(o) => {
                let url = ["file", "src", "url", "download_url"]
                    .iter()
                    .find_map(|k| o.get(*k).and_then(Value::as_str))
                    .filter(|s| s.starts_with("http"));
                match url {
                    Some(url) => out.push(MediaUrl {
                        url: url.to_string(),
                        mime: text(v, "type"),
                        label: text(v, "label"),
                    }),
                    None => o.values().for_each(|x| walk(x, depth + 1, out)),
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    if let Some(v) = v {
        walk(v, 0, &mut out);
    }
    out
}

fn captions(v: Option<&Value>) -> Vec<Caption> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|c| {
                    let url = text(c, "url").or_else(|| text(c, "file"))?;
                    Some(Caption {
                        locale: text(c, "locale_id").or_else(|| text(c, "locale")),
                        url,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_asset(v: &Value) -> Option<Asset> {
    v.is_object().then(|| Asset {
        id: id_of(v, "id"),
        asset_type: text(v, "asset_type"),
        title: text(v, "title"),
        filename: text(v, "filename"),
        status: text(v, "status"),
        time_estimation: number(v, "time_estimation"),
        is_external: boolean(v, "is_external").unwrap_or(false),
        external_url: text(v, "external_url"),
        course_is_drmed: boolean(v, "course_is_drmed"),
        has_license_token: text(v, "media_license_token").is_some(),
        streams: collect_urls(v.get("stream_urls")),
        downloads: collect_urls(v.get("download_urls")),
        sources: collect_urls(v.get("media_sources")),
        captions: captions(v.get("captions")),
        body: text(v, "body"),
    })
}

fn kind_of(v: &Value) -> ItemKind {
    match text(v, "_class").as_deref() {
        Some("chapter") => ItemKind::Chapter,
        Some("lecture") => ItemKind::Lecture,
        Some("quiz") => ItemKind::Quiz,
        Some("practice") => ItemKind::Practice,
        Some(other) => ItemKind::Other(other.to_string()),
        None => ItemKind::Other("unknown".into()),
    }
}

pub fn parse_curriculum_page(v: &Value) -> Result<CurriculumPage, UdemyError> {
    let (results, next) = envelope(v)?;
    let mut items = Vec::new();
    let mut skipped = 0;
    for item in results {
        let Some(id) = id_of(item, "id") else {
            skipped += 1;
            continue;
        };
        items.push(CurriculumItem {
            kind: kind_of(item),
            id,
            title: text(item, "title"),
            index: number(item, "object_index"),
            published: boolean(item, "is_published"),
            asset: item.get("asset").and_then(parse_asset),
            supplementary: item
                .get("supplementary_assets")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(parse_asset).collect())
                .unwrap_or_default(),
            has_description: text(item, "description").is_some(),
        });
    }
    Ok(CurriculumPage {
        items,
        next,
        skipped,
    })
}

pub fn parse_lecture_detail(v: &Value) -> Result<LectureDetail, UdemyError> {
    if !v.is_object() {
        return Err(UdemyError::Shape("the lecture is not an object".into()));
    }
    Ok(LectureDetail {
        asset: v.get("asset").and_then(parse_asset),
        description: text(v, "description"),
        download_url: text(v, "download_url"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn fixture(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    fn c1() -> Value {
        fixture(include_str!("../tests/fixtures/assumed_courses_page1.json"))
    }
    fn k1() -> Value {
        fixture(include_str!(
            "../tests/fixtures/assumed_curriculum_page1.json"
        ))
    }
    fn k2() -> Value {
        fixture(include_str!(
            "../tests/fixtures/assumed_curriculum_page2.json"
        ))
    }

    #[test]
    fn a_courses_page_is_read_tolerantly() {
        let p = parse_courses_page(&c1()).unwrap();
        assert_eq!(p.count, Some(3));
        assert!(p.next.as_deref().unwrap().contains("page=2"));
        assert_eq!(p.skipped, 0);
        let ids: Vec<_> = p.courses.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["1001", "1002"]);
        let c = &p.courses[0];
        assert_eq!(c.title.as_deref(), Some("Curso de Prueba Uno"));
        assert_eq!(
            c.cover_url.as_deref(),
            Some("https://img-c.udemycdn.com/course/480x270/1001_aaaa.jpg")
        );
        assert_eq!(c.lectures, Some(42));
        assert_eq!(c.instructors, ["Ana Ejemplo"]);
        let second = &p.courses[1];
        assert_eq!((second.cover_url.clone(), second.lectures), (None, Some(7)));
        assert!(second.instructors.is_empty());
    }

    #[test]
    fn items_without_an_id_are_skipped_and_counted_not_fatal() {
        let p = parse_courses_page(&fixture(include_str!(
            "../tests/fixtures/assumed_courses_page2.json"
        )))
        .unwrap();
        assert_eq!(p.skipped, 1);
        assert_eq!(p.next, None);
        assert_eq!(p.courses.len(), 1);
        let c = &p.courses[0];
        assert_eq!(
            (c.id.as_str(), c.title.clone(), c.lectures),
            ("1003", None, None)
        );
        let mixed = parse_courses_page(&json!({"results": [1, "x", null, {"id": 5}]})).unwrap();
        assert_eq!((mixed.courses.len(), mixed.skipped), (1, 3));
    }

    #[test]
    fn a_response_without_results_is_a_shape_error() {
        assert!(matches!(
            parse_courses_page(&json!({"detail": "x"})),
            Err(UdemyError::Shape(_))
        ));
        assert!(matches!(
            parse_courses_page(&json!([1])),
            Err(UdemyError::Shape(_))
        ));
        assert!(matches!(
            parse_curriculum_page(&json!("x")),
            Err(UdemyError::Shape(_))
        ));
    }

    #[test]
    fn course_summaries_serialize_camel_case() {
        let c = &parse_courses_page(&c1()).unwrap().courses[0];
        let v = serde_json::to_value(c).unwrap();
        assert_eq!(
            v["coverUrl"],
            "https://img-c.udemycdn.com/course/480x270/1001_aaaa.jpg"
        );
        assert_eq!(v["lectures"], 42);
        assert_eq!(v["instructors"][0], "Ana Ejemplo");
    }

    #[test]
    fn curriculum_items_keep_their_kinds_in_order() {
        let p1 = parse_curriculum_page(&k1()).unwrap();
        let kinds: Vec<_> = p1.items.iter().map(|i| i.kind.clone()).collect();
        use ItemKind::*;
        assert_eq!(
            kinds,
            [Chapter, Lecture, Lecture, Chapter, Lecture, Lecture, Quiz]
        );
        assert!(p1.next.as_deref().unwrap().contains("page=2"));
        let p2 = parse_curriculum_page(&k2()).unwrap();
        let kinds: Vec<_> = p2.items.iter().map(|i| i.kind.clone()).collect();
        assert_eq!(kinds, [Practice, Lecture, Other("mystery-item".into())]);
        assert_eq!(p2.next, None);
    }

    #[test]
    fn a_video_lecture_exposes_streams_captions_sources_and_resources() {
        let p = parse_curriculum_page(&k1()).unwrap();
        let l = &p.items[1];
        assert_eq!(
            (l.id.as_str(), l.title.as_deref(), l.index, l.published),
            ("11", Some("Bienvenida"), Some(1), Some(true))
        );
        assert!(l.has_description);
        let a = l.asset.as_ref().unwrap();
        assert_eq!(a.asset_type.as_deref(), Some("Video"));
        assert_eq!(a.filename.as_deref(), Some("intro.mp4"));
        assert_eq!(a.time_estimation, Some(120));
        assert_eq!(a.streams.len(), 2);
        assert_eq!(a.streams[0].mime.as_deref(), Some("video/mp4"));
        assert_eq!(a.streams[0].label.as_deref(), Some("720"));
        assert!(a.streams[0].url.starts_with("https://mp4.udemycdn.com/"));
        assert!(a.downloads.is_empty());
        assert_eq!(a.captions.len(), 1);
        assert_eq!(a.captions[0].locale.as_deref(), Some("es_ES"));
        assert_eq!(a.sources.len(), 1);
        assert!(!a.has_license_token && a.course_is_drmed == Some(false));
        assert_eq!(l.supplementary.len(), 1);
        let r = &l.supplementary[0];
        assert_eq!(
            (r.asset_type.as_deref(), r.filename.as_deref()),
            (Some("File"), Some("guia.pdf"))
        );
        assert_eq!(r.downloads.len(), 1);
    }

    #[test]
    fn drm_article_and_external_assets_are_recognised_by_their_fields() {
        let p = parse_curriculum_page(&k1()).unwrap();
        let article = p.items[2].asset.as_ref().unwrap();
        assert_eq!(article.body.as_deref(), Some("<p>Texto de la lectura</p>"));
        let drm = p.items[4].asset.as_ref().unwrap();
        assert!(drm.has_license_token);
        assert_eq!(drm.course_is_drmed, Some(true));
        assert_eq!(drm.sources[0].mime.as_deref(), Some("application/dash+xml"));
        let ext = p.items[5].asset.as_ref().unwrap();
        assert!(ext.is_external);
        assert_eq!(
            ext.external_url.as_deref(),
            Some("https://example.com/recurso")
        );
    }

    #[test]
    fn a_null_asset_and_unknown_classes_do_not_break_the_page() {
        let p = parse_curriculum_page(&k2()).unwrap();
        assert!(p.items[1].asset.is_none());
        assert_eq!(p.items[2].title.as_deref(), Some("Algo nuevo"));
        assert_eq!(p.skipped, 0);
    }

    #[test]
    fn wrong_types_degrade_to_none_instead_of_failing() {
        let p = parse_curriculum_page(&json!({"results": [
            {"_class": "lecture", "id": 5, "title": 7, "asset": "oops", "is_published": "yes",
             "object_index": "3", "supplementary_assets": "none"},
            {"_class": "lecture", "id": 6, "asset": {"asset_type": 3, "stream_urls": ["https://a.test/v.mp4"],
              "captions": "x", "media_sources": {"a": 1}, "is_external": "no"}},
            {"title": "no class and no id"}
        ]})).unwrap();
        assert_eq!(p.items.len(), 2);
        assert_eq!(p.skipped, 1);
        let a = &p.items[0];
        assert_eq!(
            (a.title.clone(), a.published, a.index),
            (None, None, Some(3))
        );
        assert!(a.asset.is_none() && a.supplementary.is_empty());
        let b = p.items[1].asset.as_ref().unwrap();
        assert_eq!(b.asset_type, None);
        assert_eq!(b.streams.len(), 1);
        assert!(b.captions.is_empty() && b.sources.is_empty() && !b.is_external);
    }

    #[test]
    fn a_lecture_detail_carries_the_asset_description_and_download_url() {
        let d = parse_lecture_detail(&fixture(include_str!(
            "../tests/fixtures/assumed_lecture_video.json"
        )))
        .unwrap();
        assert_eq!(d.description.as_deref(), Some("<p>Hola</p>"));
        assert_eq!(d.download_url, None);
        let a = d.asset.unwrap();
        assert_eq!(a.streams.len(), 1);
        assert_eq!(a.sources[0].mime.as_deref(), Some("application/x-mpegURL"));
        let drm = parse_lecture_detail(&fixture(include_str!(
            "../tests/fixtures/assumed_lecture_drm.json"
        )))
        .unwrap();
        assert!(drm.asset.unwrap().has_license_token);
        let art = parse_lecture_detail(&fixture(include_str!(
            "../tests/fixtures/assumed_lecture_article.json"
        )))
        .unwrap();
        assert!(art.asset.unwrap().body.is_some());
        let with_dl =
            parse_lecture_detail(&json!({"download_url": "https://a.test/f.zip"})).unwrap();
        assert_eq!(
            with_dl.download_url.as_deref(),
            Some("https://a.test/f.zip")
        );
        assert!(matches!(
            parse_lecture_detail(&json!(3)),
            Err(UdemyError::Shape(_))
        ));
    }
}
