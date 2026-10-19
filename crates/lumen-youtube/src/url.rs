//! Turns whatever the user pasted into a video or playlist id. The canonical
//! URL is rebuilt from the id alone, so nothing else the user typed ever
//! reaches yt-dlp.

use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Video(String),
    Playlist(String),
}

impl Target {
    pub fn id(&self) -> &str {
        match self {
            Self::Video(id) | Self::Playlist(id) => id,
        }
    }

    pub fn canonical_url(&self) -> String {
        match self {
            Self::Video(id) => format!("https://www.youtube.com/watch?v={id}"),
            Self::Playlist(id) => format!("https://www.youtube.com/playlist?list={id}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UrlError {
    /// Not a web link at all.
    #[error("that is not a valid link")]
    Invalid,
    /// A link, but not to a YouTube video or playlist.
    #[error("only YouTube video and playlist links are supported")]
    Unsupported,
}

const HOSTS: [&str; 8] = [
    "youtube.com",
    "www.youtube.com",
    "m.youtube.com",
    "music.youtube.com",
    "youtu.be",
    "www.youtu.be",
    "youtube-nocookie.com",
    "www.youtube-nocookie.com",
];

fn is_id(s: &str, len: std::ops::RangeInclusive<usize>) -> bool {
    len.contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn video_id(s: &str) -> Option<String> {
    is_id(s, 11..=11).then(|| s.to_string())
}

fn playlist_id(s: &str) -> Option<String> {
    is_id(s, 10..=64).then(|| s.to_string())
}

pub fn parse_target(input: &str) -> Result<Target, UrlError> {
    let input = input.trim();
    if input.is_empty() || input.starts_with('-') {
        return Err(UrlError::Invalid);
    }
    let with_scheme = if input.contains("://") {
        input.to_string()
    } else {
        format!("https://{input}")
    };
    let url = Url::parse(&with_scheme).map_err(|_| UrlError::Invalid)?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(UrlError::Invalid);
    }
    let host = url
        .host_str()
        .ok_or(UrlError::Invalid)?
        .to_ascii_lowercase();
    if !HOSTS.contains(&host.as_str()) || !url.username().is_empty() {
        return Err(UrlError::Unsupported);
    }
    let query = |key: &str| {
        url.query_pairs()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.into_owned())
    };
    let segments: Vec<&str> = url
        .path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).collect())
        .unwrap_or_default();
    let found = if host.ends_with("youtu.be") {
        segments
            .first()
            .and_then(|id| video_id(id))
            .map(Target::Video)
    } else {
        match segments.as_slice() {
            ["watch"] => match query("v") {
                Some(v) => video_id(&v).map(Target::Video),
                None => query("list")
                    .and_then(|l| playlist_id(&l))
                    .map(Target::Playlist),
            },
            ["playlist"] => query("list")
                .and_then(|l| playlist_id(&l))
                .map(Target::Playlist),
            ["shorts" | "embed" | "live", id] => video_id(id).map(Target::Video),
            _ => None,
        }
    };
    found.ok_or(UrlError::Unsupported)
}

#[cfg(test)]
mod tests {
    use super::*;

    const V: &str = "dQw4w9WgXcQ";
    const PL: &str = "PLrAXtmErZgOeiKm4sgNOknGvNjby9efdf";

    fn video(url: &str) -> String {
        match parse_target(url) {
            Ok(Target::Video(id)) => id,
            other => panic!("{url}: expected a video, got {other:?}"),
        }
    }

    fn playlist(url: &str) -> String {
        match parse_target(url) {
            Ok(Target::Playlist(id)) => id,
            other => panic!("{url}: expected a playlist, got {other:?}"),
        }
    }

    #[test]
    fn every_video_url_shape_normalises_to_the_id() {
        for url in [
            format!("https://www.youtube.com/watch?v={V}"),
            format!("https://youtube.com/watch?v={V}&t=42s"),
            format!("https://m.youtube.com/watch?v={V}"),
            format!("https://music.youtube.com/watch?v={V}&si=abc"),
            format!("https://youtu.be/{V}"),
            format!("https://youtu.be/{V}?t=10&si=xyz"),
            format!("https://www.youtube.com/shorts/{V}"),
            format!("https://www.youtube.com/embed/{V}"),
            format!("https://www.youtube.com/live/{V}"),
            format!("https://www.youtube-nocookie.com/embed/{V}"),
            format!("http://www.youtube.com/watch?v={V}"),
            format!("  https://youtu.be/{V}  "),
            format!("youtube.com/watch?v={V}"),
            format!("https://www.youtube.com/watch?v={V}&list={PL}&index=3"),
            format!("https://music.youtube.com/watch?v={V}&list={PL}"),
        ] {
            assert_eq!(video(&url), V, "{url}");
        }
    }

    #[test]
    fn every_playlist_url_shape_normalises_to_the_list_id() {
        for url in [
            format!("https://www.youtube.com/playlist?list={PL}"),
            format!("https://youtube.com/playlist?list={PL}&si=abc"),
            format!("https://music.youtube.com/playlist?list={PL}"),
            format!("https://www.youtube.com/watch?list={PL}"),
            format!("https://m.youtube.com/playlist?list={PL}"),
        ] {
            assert_eq!(playlist(&url), PL, "{url}");
        }
    }

    #[test]
    fn canonical_urls_are_rebuilt_from_the_id_alone() {
        assert_eq!(
            Target::Video(V.into()).canonical_url(),
            format!("https://www.youtube.com/watch?v={V}")
        );
        assert_eq!(
            Target::Playlist(PL.into()).canonical_url(),
            format!("https://www.youtube.com/playlist?list={PL}")
        );
        assert_eq!(Target::Video(V.into()).id(), V);
    }

    #[test]
    fn unparseable_input_is_an_invalid_url() {
        for bad in [
            "",
            "   ",
            "hello world",
            "--exec=rm",
            "not a url",
            "https://",
        ] {
            assert_eq!(parse_target(bad), Err(UrlError::Invalid), "{bad:?}");
        }
        assert_eq!(
            parse_target("ftp://www.youtube.com/watch?v=dQw4w9WgXcQ"),
            Err(UrlError::Invalid)
        );
    }

    #[test]
    fn other_hosts_and_non_video_pages_are_unsupported() {
        for url in [
            "https://vimeo.com/123456",
            "https://evil.com/watch?v=dQw4w9WgXcQ",
            "https://youtube.com.evil.com/watch?v=dQw4w9WgXcQ",
            "https://notyoutube.com/watch?v=dQw4w9WgXcQ",
            "https://youtube.com@evil.com/watch?v=dQw4w9WgXcQ",
            "https://www.youtube.com/",
            "https://www.youtube.com/@somechannel",
            "https://www.youtube.com/channel/UCabcdefghijklmnopqrstuv",
            "https://www.youtube.com/results?search_query=rust",
            "https://youtu.be/",
            "https://www.youtube.com/watch?v=short",
            "https://www.youtube.com/watch?v=dQw4w9WgXc%20",
            "https://www.youtube.com/watch?v=-oexec/bad/id",
            "https://www.youtube.com/playlist?list=",
            "https://www.youtube.com/playlist?list=a b",
        ] {
            assert_eq!(parse_target(url), Err(UrlError::Unsupported), "{url}");
        }
    }
}
