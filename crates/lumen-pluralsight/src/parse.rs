//! Pure parsers for Pluralsight history, table-of-contents, and clip delivery JSON.

use serde::Serialize;
use url::Url;

use crate::error::PluralsightError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CourseInfo {
    pub id: Option<String>,
    pub title: String,
    pub level: Option<String>,
    pub image_url: Option<String>,
    pub issue: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleInfo {
    pub title: String,
    pub clips: Vec<Clip>,
    pub issue: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: Option<String>,
    pub version: Option<String>,
    pub title: String,
    pub issue: Option<String>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ClipDelivery {
    pub stream_url: String,
}

impl std::fmt::Debug for ClipDelivery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClipDelivery")
            .field("has_stream_url", &!self.stream_url.is_empty())
            .finish_non_exhaustive()
    }
}

pub fn parse_history(body: &serde_json::Value) -> Result<Vec<CourseInfo>, PluralsightError> {
    let rows = body
        .get("collection")
        .and_then(serde_json::Value::as_array)
        .ok_or(PluralsightError::InvalidResponse(
            "history collection is missing or is not an array",
        ))?;

    Ok(rows
        .iter()
        .map(|row| {
            let content = row.get("content").filter(|v| v.is_object());
            let title = content
                .and_then(|v| text(v.get("title")))
                .map(|s| collapse(&s))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "Unnamed Course".into());
            let id = content
                .and_then(|v| text(v.get("courseId")))
                .or_else(|| content.and_then(|v| text(v.get("id"))));
            let level = content.and_then(|v| text(v.get("level")));
            let image_url = content
                .and_then(|v| v.get("image"))
                .and_then(|v| text(v.get("url")));
            let issue = match (content, id.as_deref()) {
                (None, _) => Some("history row has no content object".into()),
                (Some(_), None) => Some("course record is missing an id".into()),
                _ => None,
            };
            CourseInfo {
                id,
                title,
                level,
                image_url,
                issue,
            }
        })
        .collect())
}

pub fn parse_toc(body: &serde_json::Value) -> Result<Vec<ModuleInfo>, PluralsightError> {
    let modules = body
        .get("modules")
        .and_then(serde_json::Value::as_array)
        .ok_or(PluralsightError::InvalidResponse(
            "course table of contents has no modules array",
        ))?;

    Ok(modules
        .iter()
        .map(|module| {
            let object = module.is_object();
            let title = text(module.get("title"))
                .map(|s| collapse(&s))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "Module".into());
            let items = module
                .get("contentItems")
                .and_then(serde_json::Value::as_array);
            let (clips, issue) = match (object, items) {
                (false, _) => (Vec::new(), Some("module entry is not an object".into())),
                (true, None) => (Vec::new(), Some("module has no contentItems array".into())),
                (true, Some(items)) => (items.iter().map(parse_clip).collect(), None),
            };
            ModuleInfo {
                title,
                clips,
                issue,
            }
        })
        .collect())
}

pub fn parse_clip_delivery(body: &serde_json::Value) -> Result<ClipDelivery, PluralsightError> {
    if has_explicit_protection_marker(body) {
        return Err(PluralsightError::ProtectedContent);
    }
    let renditions = body
        .get("renditions")
        .and_then(serde_json::Value::as_array)
        .ok_or(PluralsightError::InvalidResponse(
            "clip delivery response has no renditions array",
        ))?;
    let candidates = renditions.iter().filter(|rendition| is_hls(rendition));

    let mut fallback = None;
    for rendition in candidates {
        let Some(url) = rendition_url(rendition) else {
            continue;
        };
        let is_english = rendition
            .get("languageCode")
            .and_then(serde_json::Value::as_str)
            .is_some_and(is_english_language);
        if is_english {
            return Ok(ClipDelivery { stream_url: url });
        }
        fallback.get_or_insert(url);
    }
    fallback
        .map(|stream_url| ClipDelivery { stream_url })
        .ok_or(PluralsightError::NoStream)
}

fn is_english_language(language: &str) -> bool {
    language.eq_ignore_ascii_case("en") || language.to_ascii_lowercase().starts_with("en-")
}

fn text(value: Option<&serde_json::Value>) -> Option<String> {
    match value? {
        serde_json::Value::String(text) => Some(text.trim().to_string()).filter(|s| !s.is_empty()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse_clip(value: &serde_json::Value) -> Clip {
    if !value.is_object() {
        return Clip {
            id: None,
            version: None,
            title: "Clip".into(),
            issue: Some("clip entry is not an object".into()),
        };
    }
    let id = text(value.get("id"));
    let version = text(value.get("version"));
    let title = text(value.get("title"))
        .map(|s| collapse(&s))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Clip".into());
    let issue = match (id.as_deref(), version.as_deref()) {
        (None, _) => Some("clip record is missing an id".into()),
        (_, None) => Some("clip record is missing a version".into()),
        _ => None,
    };
    Clip {
        id,
        version,
        title,
        issue,
    }
}

fn is_hls(rendition: &serde_json::Value) -> bool {
    rendition
        .get("format")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|format| format.eq_ignore_ascii_case("m3u8"))
}

fn rendition_url(rendition: &serde_json::Value) -> Option<String> {
    rendition
        .get("urls")?
        .as_array()?
        .iter()
        .filter_map(|entry| entry.get("url").and_then(serde_json::Value::as_str))
        .find_map(valid_media_url)
}

fn valid_media_url(raw: &str) -> Option<String> {
    let url = Url::parse(raw).ok()?;
    (url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| url.to_string())
}

fn has_explicit_protection_marker(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(object) => object.iter().any(|(key, child)| {
            let key = key.to_ascii_lowercase();
            let is_protection_key = [
                "drm",
                "encrypt",
                "protect",
                "widevine",
                "playready",
                "license",
            ]
            .iter()
            .any(|marker| key.contains(marker));
            (is_protection_key && marker_is_active(child)) || has_explicit_protection_marker(child)
        }),
        serde_json::Value::Array(items) => items.iter().any(has_explicit_protection_marker),
        _ => false,
    }
}

fn marker_is_active(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::Bool(active) => *active,
        serde_json::Value::Number(number) => number.as_i64().is_some_and(|n| n != 0),
        serde_json::Value::String(text) => {
            let text = text.trim();
            !text.is_empty()
                && !["false", "none", "no", "clear", "off"]
                    .contains(&text.to_ascii_lowercase().as_str())
        }
        serde_json::Value::Array(items) => !items.is_empty(),
        serde_json::Value::Object(object) => object.values().any(marker_is_active),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HISTORY: &str = include_str!("../tests/fixtures/history.json");
    const TOC: &str = include_str!("../tests/fixtures/toc.json");
    const DELIVERY: &str = include_str!("../tests/fixtures/delivery.json");

    fn json(raw: &str) -> serde_json::Value {
        serde_json::from_str(raw).unwrap()
    }

    #[test]
    fn parses_history_aliases_and_keeps_malformed_rows() {
        let courses = parse_history(&json(HISTORY)).unwrap();
        assert_eq!(courses.len(), 3);
        assert_eq!(courses[0].id.as_deref(), Some("course-1"));
        assert_eq!(courses[0].title, "Rust Essentials");
        assert_eq!(courses[0].level.as_deref(), Some("Intermediate"));
        assert_eq!(
            courses[0].image_url.as_deref(),
            Some("https://images.example.test/rust.jpg")
        );
        assert_eq!(courses[1].id.as_deref(), Some("course-2"));
        assert_eq!(courses[1].title, "Unnamed Course");
        assert!(courses[2].id.is_none());
        assert!(courses[2].issue.is_some(), "malformed row is retained");
    }

    #[test]
    fn rejects_malformed_history_envelopes() {
        assert!(matches!(
            parse_history(&json("{}")),
            Err(PluralsightError::InvalidResponse(_))
        ));
        assert!(matches!(
            parse_history(&json(r#"{"collection":{}}"#)),
            Err(PluralsightError::InvalidResponse(_))
        ));
    }

    #[test]
    fn parses_toc_and_retains_invalid_clip_rows_for_reporting() {
        let modules = parse_toc(&json(TOC)).unwrap();
        assert_eq!(modules.len(), 2);
        assert_eq!(modules[0].title, "Getting Started");
        assert_eq!(modules[0].clips.len(), 3);
        assert_eq!(modules[0].clips[0].id.as_deref(), Some("clip-1"));
        assert_eq!(modules[0].clips[0].version.as_deref(), Some("v1"));
        assert_eq!(modules[0].clips[1].title, "Clip");
        assert!(modules[0].clips[2].issue.is_some());
        assert_eq!(modules[1].title, "Module");
        assert!(modules[1].clips[0].issue.is_some());
    }

    #[test]
    fn rejects_malformed_toc_envelopes() {
        assert!(matches!(
            parse_toc(&json("{}")),
            Err(PluralsightError::InvalidResponse(_))
        ));
        assert!(matches!(
            parse_toc(&json(r#"{"modules":{}}"#)),
            Err(PluralsightError::InvalidResponse(_))
        ));
    }

    #[test]
    fn prefers_english_hls_and_redacts_signed_delivery_urls() {
        let delivery = parse_clip_delivery(&json(DELIVERY)).unwrap();
        assert_eq!(
            delivery.stream_url,
            "https://media.example.test/en.m3u8?signature=synthetic"
        );
        assert!(!format!("{delivery:?}").contains("signature"));
    }

    #[test]
    fn falls_back_to_valid_hls_and_rejects_unsafe_or_missing_streams() {
        let fallback = json(
            r#"{"renditions":[{"format":"m3u8","languageCode":"pt-BR","urls":[{"url":"https://media.example.test/other.m3u8"}]}]}"#,
        );
        assert_eq!(
            parse_clip_delivery(&fallback).unwrap().stream_url,
            "https://media.example.test/other.m3u8"
        );
        for body in [
            r#"{"renditions":[{"format":"m3u8","urls":[{"url":"http://media.example.test/x.m3u8"}]}]}"#,
            r#"{"renditions":[{"format":"m3u8","urls":[{"url":"not a URL"}]}]}"#,
            r#"{"renditions":[{"format":"mp4","urls":[{"url":"https://media.example.test/x.mp4"}]}]}"#,
        ] {
            assert!(matches!(
                parse_clip_delivery(&json(body)),
                Err(PluralsightError::NoStream)
            ));
        }
        assert!(matches!(
            parse_clip_delivery(&json("{}")),
            Err(PluralsightError::InvalidResponse(_))
        ));
    }

    #[test]
    fn rejects_explicit_protection_markers() {
        let protected = json(
            r#"{"encrypted":true,"renditions":[{"format":"m3u8","urls":[{"url":"https://media.example.test/x.m3u8"}]}]}"#,
        );
        assert!(matches!(
            parse_clip_delivery(&protected),
            Err(PluralsightError::ProtectedContent)
        ));
    }

    #[test]
    fn serializes_public_parser_models_as_camel_case() {
        let course = CourseInfo {
            id: Some("c".into()),
            title: "Course".into(),
            level: None,
            image_url: Some("https://images.example.test/c.jpg".into()),
            issue: None,
        };
        let encoded = serde_json::to_value(course).unwrap();
        assert_eq!(encoded["imageUrl"], "https://images.example.test/c.jpg");
        assert!(encoded.get("image_url").is_none());
    }
}
