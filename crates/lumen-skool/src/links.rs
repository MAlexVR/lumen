//! Decisions about links: which are files worth downloading and which
//! extension a downloaded file gets.

use std::sync::OnceLock;

use regex::Regex;
use url::Url;

/// Host serving uploaded Skool files (`assets.skool.com`), not pages or API.
pub fn is_skool_file_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host.ends_with(".skool.com") && host != "www.skool.com" && host != "api2.skool.com"
}

/// Hosts that never serve a plain file (kept as `.txt` links).
pub fn is_excluded_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    ["google.com", "forms.gle", "youtube.com", "youtu.be"]
        .iter()
        .any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

/// Whether a resource/description link should be fetched as a file.
pub fn is_downloadable_link(link: &str) -> bool {
    let Ok(url) = Url::parse(link) else {
        return false;
    };
    if url.scheme() != "http" && url.scheme() != "https" {
        return false;
    }
    let host = url.host_str().unwrap_or("");
    if is_excluded_host(host) {
        return false;
    }
    is_skool_file_host(host) || known_extension(url.path()).is_some()
}

const FILE_EXTS: [&str; 30] = [
    ".pdf", ".zip", ".rar", ".7z", ".doc", ".docx", ".xls", ".xlsx", ".xlsm", ".csv", ".ppt",
    ".pptx", ".key", ".pages", ".numbers", ".epub", ".mobi", ".psd", ".ai", ".fig", ".png", ".jpg",
    ".jpeg", ".gif", ".webp", ".mp3", ".wav", ".m4a", ".mp4", ".mov",
];

const TYPE_EXTS: [(&str, &str); 19] = [
    ("application/pdf", ".pdf"),
    ("application/zip", ".zip"),
    ("application/x-rar-compressed", ".rar"),
    ("application/vnd.rar", ".rar"),
    ("application/x-7z-compressed", ".7z"),
    ("application/msword", ".doc"),
    ("application/vnd.ms-excel", ".xls"),
    ("application/vnd.ms-powerpoint", ".ppt"),
    (
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ".docx",
    ),
    (
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ".xlsx",
    ),
    (
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        ".pptx",
    ),
    ("application/epub+zip", ".epub"),
    ("text/csv", ".csv"),
    ("image/jpeg", ".jpg"),
    ("image/png", ".png"),
    ("image/gif", ".gif"),
    ("image/webp", ".webp"),
    ("audio/mpeg", ".mp3"),
    ("video/mp4", ".mp4"),
];

/// Lowercase extension of a URL path when it is a known file type.
fn known_extension(path: &str) -> Option<String> {
    let name = path.rsplit('/').next()?;
    let ext = format!(".{}", name.rsplit_once('.')?.1.to_ascii_lowercase());
    FILE_EXTS.contains(&ext.as_str()).then_some(ext)
}

fn disposition_filename(disposition: &str) -> Option<String> {
    let re = DISPOSITION.get_or_init(|| {
        Regex::new(r#"(?i)filename\*?=(?:UTF-8'')?"?([^";]+)"?"#)
            .unwrap_or_else(|_| unreachable!("static regex"))
    });
    re.captures(disposition).map(|c| c[1].trim().to_string())
}

static DISPOSITION: OnceLock<Regex> = OnceLock::new();

/// Extension (with dot, lowercase) from the URL, the Content-Disposition
/// filename or the Content-Type, in that order; `.bin` when unknown.
pub fn file_extension(link: &str, content_type: Option<&str>, disposition: Option<&str>) -> String {
    if let Some(ext) = Url::parse(link)
        .ok()
        .and_then(|u| known_extension(u.path()))
    {
        return ext;
    }
    if let Some(name) = disposition.and_then(disposition_filename) {
        if let Some((_, ext)) = name.rsplit_once('.') {
            let ext = ext.to_ascii_lowercase();
            if !ext.is_empty() && ext.len() <= 5 && ext.chars().all(|c| c.is_ascii_alphanumeric()) {
                return format!(".{ext}");
            }
        }
    }
    let kind = content_type
        .and_then(|c| c.split(';').next())
        .map(|c| c.trim().to_ascii_lowercase())
        .unwrap_or_default();
    TYPE_EXTS
        .iter()
        .find(|(t, _)| *t == kind)
        .map_or_else(|| ".bin".to_string(), |(_, e)| (*e).to_string())
}

/// Strips tracking query strings only where it is safe (Vimeo). Mux needs
/// `token=` and YouTube needs `watch?v=`.
pub fn clean_video_url(link: &str) -> String {
    if link.contains("vimeo.com") {
        link.split('?').next().unwrap_or(link).to_string()
    } else {
        link.to_string()
    }
}

/// Mux and Vimeo reject requests without the Skool referer.
pub fn needs_skool_referer(link: &str) -> bool {
    link.contains("mux.com") || link.contains("vimeo.com")
}

/// File name without extension taken from the URL path (fallback title).
pub fn url_file_stem(link: &str) -> String {
    let path = Url::parse(link)
        .map(|u| u.path().to_string())
        .unwrap_or_default();
    let name = path.rsplit('/').next().unwrap_or("");
    name.rsplit_once('.')
        .map_or(name, |(stem, _)| stem)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skool_file_hosts() {
        assert!(is_skool_file_host("assets.skool.com"));
        assert!(is_skool_file_host("Files.Skool.com"));
        assert!(!is_skool_file_host("www.skool.com"));
        assert!(!is_skool_file_host("api2.skool.com"));
        assert!(!is_skool_file_host("skool.com"));
        assert!(!is_skool_file_host("evilskool.com"));
    }

    #[test]
    fn downloadable_by_extension_or_skool_host() {
        assert!(is_downloadable_link("https://x.test/a.PDF?x=1"));
        assert!(is_downloadable_link("https://assets.skool.com/f/abc"));
        assert!(is_downloadable_link("http://x.test/a/b.zip"));
        assert!(!is_downloadable_link("https://x.test/page"));
        assert!(!is_downloadable_link("https://x.test/page.html"));
        assert!(!is_downloadable_link("ftp://x.test/a.pdf"));
        assert!(!is_downloadable_link("not a url"));
        assert!(!is_downloadable_link("https://www.skool.com/g/classroom"));
    }

    #[test]
    fn google_and_youtube_links_are_never_downloaded() {
        for u in [
            "https://drive.google.com/file/d/1/view.pdf",
            "https://docs.google.com/document/d/1/export.pdf",
            "https://forms.gle/abc.pdf",
            "https://www.youtube.com/watch?v=1",
            "https://youtu.be/abc.mp4",
        ] {
            assert!(!is_downloadable_link(u), "{u}");
        }
        assert!(is_excluded_host("drive.google.com"));
        assert!(!is_excluded_host("assets.skool.com"));
    }

    #[test]
    fn extension_precedence_url_then_disposition_then_type() {
        assert_eq!(
            file_extension("https://x.test/a.PDF?z=1", None, None),
            ".pdf"
        );
        assert_eq!(
            file_extension(
                "https://assets.skool.com/f/abc",
                None,
                Some(r#"attachment; filename="Report.XLSX""#)
            ),
            ".xlsx"
        );
        assert_eq!(
            file_extension(
                "https://assets.skool.com/f/abc",
                None,
                Some("attachment; filename*=UTF-8''guia.pdf")
            ),
            ".pdf"
        );
        assert_eq!(
            file_extension(
                "https://x.test/f",
                Some("application/pdf; charset=binary"),
                None
            ),
            ".pdf"
        );
        assert_eq!(
            file_extension("https://x.test/f.php", Some("text/csv"), None),
            ".csv"
        );
        assert_eq!(file_extension("https://x.test/f", None, None), ".bin");
        assert_eq!(
            file_extension(
                "https://x.test/f",
                None,
                Some(r#"attachment; filename="a.toolongext""#)
            ),
            ".bin"
        );
    }

    #[test]
    fn only_vimeo_urls_lose_their_query() {
        assert_eq!(
            clean_video_url("https://vimeo.com/123?share=copy"),
            "https://vimeo.com/123"
        );
        let yt = "https://www.youtube.com/watch?v=abc&t=3";
        assert_eq!(clean_video_url(yt), yt);
        let mux = "https://stream.mux.com/p.m3u8?token=t";
        assert_eq!(clean_video_url(mux), mux);
    }

    #[test]
    fn referer_is_needed_for_mux_and_vimeo_only() {
        assert!(needs_skool_referer("https://stream.mux.com/p.m3u8?token=t"));
        assert!(needs_skool_referer("https://player.vimeo.com/video/1"));
        assert!(!needs_skool_referer("https://www.youtube.com/watch?v=1"));
    }

    #[test]
    fn url_stem_is_the_file_name_without_extension() {
        assert_eq!(url_file_stem("https://x.test/a/guide.pdf?z=1"), "guide");
        assert_eq!(url_file_stem("https://x.test/a/noext"), "noext");
        assert_eq!(url_file_stem("https://x.test/"), "");
    }
}
