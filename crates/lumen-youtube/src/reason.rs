//! Classifies yt-dlp failure messages so the report says why a video could
//! not be downloaded instead of dumping a raw error.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reason {
    pub text: &'static str,
    /// True when retrying later cannot help (private, removed, ...).
    pub permanent: bool,
}

/// Ordered: the first matching rule wins, so specific ones come first.
const RULES: [(&[&str], &str, bool); 10] = [
    (
        &["private video", "video is private"],
        "private video",
        true,
    ),
    (
        &[
            "confirm your age",
            "age-restricted",
            "inappropriate for some users",
        ],
        "age-restricted video",
        true,
    ),
    (
        &[
            "available in your country",
            "blocked it in your country",
            "blocked in your country",
        ],
        "not available in your region",
        true,
    ),
    (
        &["members-only", "join this channel", "members on level"],
        "members-only video",
        true,
    ),
    (
        &["not a bot"],
        "YouTube asked for sign-in (bot check)",
        false,
    ),
    (
        &["live event will begin", "premieres in"],
        "live event not started",
        true,
    ),
    (
        &[
            "has been removed",
            "no longer available",
            "has been terminated",
        ],
        "removed video",
        true,
    ),
    (&["video unavailable"], "removed or unavailable video", true),
    (
        &["this video is unavailable"],
        "removed or unavailable video",
        true,
    ),
    (
        &["video is not available"],
        "removed or unavailable video",
        true,
    ),
];

pub fn classify(message: &str) -> Option<Reason> {
    let lower = message.to_lowercase();
    RULES
        .iter()
        .find(|(needles, _, _)| needles.iter().any(|n| lower.contains(n)))
        .map(|(_, text, permanent)| Reason {
            text,
            permanent: *permanent,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yt_dlp_messages_are_classified_by_reason() {
        let cases = [
            ("[youtube] abc: Video unavailable. This video is private", "private video"),
            ("Private video. Sign in if you've been granted access to this video", "private video"),
            ("[youtube] abc: Sign in to confirm your age. This video may be inappropriate for some users.", "age-restricted video"),
            ("The uploader has not made this video available in your country", "not available in your region"),
            ("This video is not available in your country", "not available in your region"),
            ("[youtube] abc: This video is available to this channel's members on level: Gold. Join this channel to get access", "members-only video"),
            ("Video unavailable. This video has been removed by the uploader", "removed video"),
            ("[youtube] abc: Video unavailable", "removed or unavailable video"),
            ("This video is no longer available because the YouTube account associated with this video has been terminated.", "removed video"),
            ("Sign in to confirm you're not a bot", "YouTube asked for sign-in (bot check)"),
            ("This live event will begin in 2 hours", "live event not started"),
        ];
        for (msg, want) in cases {
            let r = classify(msg).unwrap_or_else(|| panic!("{msg}"));
            assert_eq!(r.text, want, "{msg}");
            assert!(
                r.permanent == (want != "YouTube asked for sign-in (bot check)"),
                "{msg}"
            );
        }
    }

    #[test]
    fn unknown_failures_are_not_classified() {
        assert_eq!(classify("HTTP Error 503: Service Unavailable"), None);
        assert_eq!(classify("unable to download video data"), None);
    }
}
