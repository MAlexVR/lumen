//! Pure parsers over the JSON `yt-dlp -J` prints. Playlists come from
//! `--flat-playlist`: entries are not resolved, so private or removed videos
//! show up as placeholders that are kept (never dropped) and flagged.

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unexpected yt-dlp output: {0}")]
pub struct ParseError(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoInfo {
    pub id: String,
    pub title: String,
    pub channel: String,
    pub thumbnail_url: Option<String>,
    pub duration_secs: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemInfo {
    pub id: String,
    pub title: String,
    pub duration_secs: Option<u64>,
    pub thumbnail_url: Option<String>,
    pub available: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistInfo {
    pub id: String,
    pub title: String,
    pub channel: String,
    pub thumbnail_url: Option<String>,
    /// Every playlist position, in order; unavailable ones are flagged.
    pub items: Vec<ItemInfo>,
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn duration(v: &Value) -> Option<u64> {
    let d = v.get("duration")?.as_f64()?;
    (d >= 0.0).then(|| d.round() as u64)
}

/// `thumbnail`, else the last (largest) of `thumbnails`.
fn thumbnail(v: &Value) -> Option<String> {
    text(v, "thumbnail").map(str::to_string).or_else(|| {
        v.get("thumbnails")?
            .as_array()?
            .iter()
            .rev()
            .find_map(|t| text(t, "url"))
            .map(str::to_string)
    })
}

pub fn parse_video(v: &Value) -> Result<VideoInfo, ParseError> {
    let id = text(v, "id").ok_or_else(|| ParseError("the video has no id".into()))?;
    let title = text(v, "title").ok_or_else(|| ParseError("the video has no title".into()))?;
    Ok(VideoInfo {
        id: id.to_string(),
        title: title.to_string(),
        channel: text(v, "channel")
            .or_else(|| text(v, "uploader"))
            .unwrap_or_default()
            .to_string(),
        thumbnail_url: thumbnail(v),
        duration_secs: duration(v),
    })
}

/// Why a flat entry cannot be downloaded, if it cannot.
fn unavailable_reason(title: &str, availability: Option<&str>) -> Option<&'static str> {
    match (title, availability) {
        ("[Private video]", _) | (_, Some("private")) => Some("private video"),
        ("[Deleted video]", _) => Some("removed video"),
        (_, Some("needs_auth")) => {
            Some("requires sign-in (private, members-only or age-restricted)")
        }
        (_, Some("subscriber_only" | "premium_only")) => Some("members-only video"),
        (t, _)
            if t.starts_with('[')
                && t.ends_with(']')
                && t.to_ascii_lowercase().contains("unavailable") =>
        {
            Some("unavailable video")
        }
        _ => None,
    }
}

fn parse_item(position: usize, v: &Value) -> ItemInfo {
    let placeholder = |reason: &str| ItemInfo {
        id: format!("unavailable-{position}"),
        title: "Unavailable video".to_string(),
        duration_secs: None,
        thumbnail_url: None,
        available: false,
        reason: Some(reason.to_string()),
    };
    let (Some(id), Some(title)) = (text(v, "id"), text(v, "title")) else {
        return placeholder("unavailable video");
    };
    let reason = unavailable_reason(title, text(v, "availability"));
    ItemInfo {
        id: id.to_string(),
        title: title.to_string(),
        duration_secs: duration(v),
        thumbnail_url: thumbnail(v),
        available: reason.is_none(),
        reason: reason.map(str::to_string),
    }
}

pub fn parse_playlist(v: &Value) -> Result<PlaylistInfo, ParseError> {
    let title = text(v, "title").ok_or_else(|| ParseError("the playlist has no title".into()))?;
    let entries = v
        .get("entries")
        .and_then(Value::as_array)
        .ok_or_else(|| ParseError("the playlist has no entries".into()))?;
    Ok(PlaylistInfo {
        id: text(v, "id").unwrap_or_default().to_string(),
        title: title.to_string(),
        channel: text(v, "channel")
            .or_else(|| text(v, "uploader"))
            .unwrap_or_default()
            .to_string(),
        thumbnail_url: thumbnail(v),
        items: entries
            .iter()
            .enumerate()
            .map(|(i, e)| parse_item(i + 1, e))
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_video_document_gives_the_preview_fields() {
        let v = parse_video(&json!({
            "id": "dQw4w9WgXcQ", "title": "Canción ñ", "channel": "Canal",
            "uploader": "Otro", "thumbnail": "https://i.ytimg.com/vi/x/hq.jpg", "duration": 212.0
        }))
        .unwrap();
        assert_eq!(v.id, "dQw4w9WgXcQ");
        assert_eq!(v.title, "Canción ñ");
        assert_eq!(v.channel, "Canal");
        assert_eq!(
            v.thumbnail_url.as_deref(),
            Some("https://i.ytimg.com/vi/x/hq.jpg")
        );
        assert_eq!(v.duration_secs, Some(212));
    }

    #[test]
    fn a_video_falls_back_to_uploader_and_tolerates_missing_optionals() {
        let v = parse_video(&json!({"id": "abcdefghijk", "title": "T", "uploader": "Up"})).unwrap();
        assert_eq!(v.channel, "Up");
        assert_eq!((v.thumbnail_url, v.duration_secs), (None, None));
        let v = parse_video(&json!({"id": "abcdefghijk", "title": "T"})).unwrap();
        assert_eq!(v.channel, "");
        assert!(matches!(
            parse_video(&json!({"title": "T"})),
            Err(ParseError(_))
        ));
        assert!(matches!(
            parse_video(&json!({"id": "abcdefghijk"})),
            Err(ParseError(_))
        ));
        assert!(matches!(parse_video(&json!([1])), Err(ParseError(_))));
    }

    fn flat() -> serde_json::Value {
        json!({
            "_type": "playlist", "id": "PLxxxxxxxxxxxx", "title": "Mi lista", "channel": "Canal",
            "thumbnails": [{"url": "https://i.ytimg.com/pl/small.jpg"}, {"url": "https://i.ytimg.com/pl/big.jpg"}],
            "entries": [
                {"id": "aaaaaaaaaaa", "title": "Uno", "duration": 61.0,
                 "thumbnails": [{"url": "https://i.ytimg.com/vi/a/1.jpg"}, {"url": "https://i.ytimg.com/vi/a/2.jpg"}]},
                {"id": "bbbbbbbbbbb", "title": "[Private video]", "duration": null},
                {"id": "ccccccccccc", "title": "[Deleted video]"},
                null,
                {"id": "ddddddddddd", "title": "Cuatro", "availability": "needs_auth"},
                {"id": "eeeeeeeeeee", "title": "Cinco", "availability": "subscriber_only"},
                {"id": "fffffffffff", "title": "Seis", "availability": "public", "thumbnail": "https://i.ytimg.com/vi/f/1.jpg"}
            ]
        })
    }

    #[test]
    fn a_flat_playlist_keeps_every_position_and_marks_unavailable_ones() {
        let p = parse_playlist(&flat()).unwrap();
        assert_eq!(
            (p.id.as_str(), p.title.as_str(), p.channel.as_str()),
            ("PLxxxxxxxxxxxx", "Mi lista", "Canal")
        );
        assert_eq!(
            p.thumbnail_url.as_deref(),
            Some("https://i.ytimg.com/pl/big.jpg")
        );
        assert_eq!(p.items.len(), 7);
        let ok = &p.items[0];
        assert_eq!(
            (ok.id.as_str(), ok.title.as_str(), ok.duration_secs),
            ("aaaaaaaaaaa", "Uno", Some(61))
        );
        assert!(ok.available && ok.reason.is_none());
        assert_eq!(
            ok.thumbnail_url.as_deref(),
            Some("https://i.ytimg.com/vi/a/2.jpg")
        );
        let reasons: Vec<_> = p
            .items
            .iter()
            .map(|i| (i.available, i.reason.as_deref()))
            .collect();
        assert_eq!(
            reasons,
            [
                (true, None),
                (false, Some("private video")),
                (false, Some("removed video")),
                (false, Some("unavailable video")),
                (
                    false,
                    Some("requires sign-in (private, members-only or age-restricted)")
                ),
                (false, Some("members-only video")),
                (true, None),
            ]
        );
        assert_eq!(p.items[3].id, "unavailable-4");
        assert_eq!(
            p.items[6].thumbnail_url.as_deref(),
            Some("https://i.ytimg.com/vi/f/1.jpg")
        );
    }

    #[test]
    fn playlist_documents_without_title_or_entries_are_errors_but_empty_is_allowed() {
        assert!(matches!(
            parse_playlist(&json!({"entries": []})),
            Err(ParseError(_))
        ));
        assert!(matches!(
            parse_playlist(&json!({"title": "T"})),
            Err(ParseError(_))
        ));
        let p = parse_playlist(&json!({"id": "PL1", "title": "T", "entries": []})).unwrap();
        assert!(p.items.is_empty());
        assert_eq!(p.channel, "");
    }

    #[test]
    fn items_serialize_camel_case() {
        let p = parse_playlist(&flat()).unwrap();
        let v = serde_json::to_value(&p.items[1]).unwrap();
        assert_eq!(v["durationSecs"], serde_json::Value::Null);
        assert_eq!(v["thumbnailUrl"], serde_json::Value::Null);
        assert_eq!(v["available"], false);
        assert_eq!(v["reason"], "private video");
    }
}
