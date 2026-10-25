//! PandaVideo embeds. A player URL
//! `https://player-vz-<library>.tv.pandavideo.com.br/embed/?v=<id>[&...]`
//! maps to the plain (unencrypted, DRM-free) HLS playlist
//! `https://b-vz-<library>.tv.pandavideo.com.br/<id>/playlist.m3u8`.

const PLAYER_PREFIX: &str = "player-vz-";
const HOST_SUFFIX: &str = ".tv.pandavideo.com.br";

fn is_id(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// HLS playlist of a Panda player URL, or `None` when `embed_url` is not a
/// Panda player URL with a valid `v` parameter. Extra query parameters
/// (`color`, `autoplay`, ...) are ignored.
pub fn playlist_url(embed_url: &str) -> Option<String> {
    let rest = embed_url
        .strip_prefix("https://")
        .or_else(|| embed_url.strip_prefix("http://"))?;
    let (host, path_query) = rest.split_once('/')?;
    let library = host
        .strip_prefix(PLAYER_PREFIX)?
        .strip_suffix(HOST_SUFFIX)?;
    if !is_id(library) {
        return None;
    }
    let (_, query) = path_query.split_once('?')?;
    let video = query
        .split(['&', '#'])
        .find_map(|pair| pair.strip_prefix("v="))?;
    is_id(video).then(|| format!("https://b-vz-{library}{HOST_SUFFIX}/{video}/playlist.m3u8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_player_url_becomes_its_playlist() {
        assert_eq!(
            playlist_url(
                "https://player-vz-a02515ca-43f.tv.pandavideo.com.br/embed/?v=36a394ff-5c24-4b0b-a9c8-4de2be22cf4f"
            )
            .as_deref(),
            Some("https://b-vz-a02515ca-43f.tv.pandavideo.com.br/36a394ff-5c24-4b0b-a9c8-4de2be22cf4f/playlist.m3u8")
        );
    }

    #[test]
    fn extra_query_parameters_do_not_leak_into_the_playlist_path() {
        assert_eq!(
            playlist_url(
                "https://player-vz-x1.tv.pandavideo.com.br/embed/?color=af8c5a&v=abc-1&autoplay=1"
            )
            .as_deref(),
            Some("https://b-vz-x1.tv.pandavideo.com.br/abc-1/playlist.m3u8")
        );
    }

    #[test]
    fn anything_else_is_rejected() {
        for url in [
            "https://example.com/embed/?v=abc",
            "https://player-vz-x1.tv.pandavideo.com.br/embed/",
            "https://player-vz-x1.tv.pandavideo.com.br/embed/?v=",
            "https://player-vz-x1.tv.pandavideo.com.br/embed/?v=a/../b",
            "https://player-vz-x1.evil.com/embed/?v=abc",
            "https://player-vz-x/1.tv.pandavideo.com.br/embed/?v=abc",
            "ftp://player-vz-x1.tv.pandavideo.com.br/embed/?v=abc",
        ] {
            assert_eq!(playlist_url(url), None, "{url}");
        }
    }
}
