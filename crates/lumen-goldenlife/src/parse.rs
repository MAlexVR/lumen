//! Pure parsers over the Cademí HTML pages (fixture-tested, no I/O).

use std::sync::OnceLock;

use lumen_download::panda::playlist_url;
use regex::Regex;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CourseInfo {
    pub id: String,
    pub title: String,
    /// Academy section (shelf) the course is listed under.
    pub shelf: Option<String>,
    pub image_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LessonRef {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub id: String,
    pub title: String,
    pub lessons: Vec<LessonRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Outline {
    /// Product the sidebar belongs to (differs from the requested course
    /// when the course is an alias of another one).
    pub product_id: Option<String>,
    pub sections: Vec<Section>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedKind {
    Panda,
    YouTube,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Embed {
    pub kind: EmbedKind,
    /// URL yt-dlp downloads: the HLS playlist (Panda) or the watch URL.
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    pub name: String,
    /// Site-relative `/area/arquivo/anexo?...` path (signed by the platform).
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LessonPage {
    pub title: String,
    pub main_video: Option<Embed>,
    /// The platform shows "URL no identificada" where the video should be.
    pub video_missing: bool,
    pub content_html: String,
    pub embeds: Vec<Embed>,
    pub attachments: Vec<Attachment>,
}

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).unwrap_or_else(|_| unreachable!("static regex")))
}

/// Decodes the HTML entities Cademí emits (named basics and numeric ones).
fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let decoded = rest.find(';').filter(|end| *end <= 10).and_then(|end| {
            let name = &rest[1..end];
            let c = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ => name.strip_prefix('#').and_then(|n| {
                    let code = match n.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok(),
                        None => n.parse().ok(),
                    };
                    code.and_then(char::from_u32)
                }),
            };
            c.map(|c| (c, end))
        });
        match decoded {
            Some((c, end)) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Entity-decoded text with runs of whitespace collapsed.
fn clean_text(text: &str) -> String {
    decode_entities(text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("{name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let len = tag[start..].find('"')?;
    Some(&tag[start..start + len])
}

pub fn parse_login_token(html: &str) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let tag = re(&RE, r#"<input[^>]*name="_token"[^>]*>"#).find(html)?;
    attr(tag.as_str(), "value")
        .map(decode_entities)
        .filter(|token| !token.is_empty())
}

pub fn is_login_page(html: &str) -> bool {
    html.contains(r#"name="Acesso[senha]""#)
}

pub fn parse_home(html: &str) -> Vec<CourseInfo> {
    static VITRINE: OnceLock<Regex> = OnceLock::new();
    static TITLE: OnceLock<Regex> = OnceLock::new();
    static PRODUCT: OnceLock<Regex> = OnceLock::new();
    static IMAGE: OnceLock<Regex> = OnceLock::new();
    let starts: Vec<usize> = re(&VITRINE, r#"<div class="vitrine""#)
        .find_iter(html)
        .map(|m| m.start())
        .collect();
    let mut courses: Vec<CourseInfo> = Vec::new();
    for (i, start) in starts.iter().enumerate() {
        let end = starts.get(i + 1).copied().unwrap_or(html.len());
        let block = &html[*start..end];
        let shelf = re(
            &TITLE,
            r#"(?s)class="main-title"[^>]*>\s*<span>(.*?)</span>"#,
        )
        .captures(block)
        .map(|c| clean_text(&c[1]))
        .filter(|t| !t.is_empty());
        let products = re(
            &PRODUCT,
            r#"(?s)<a\s+href="/area/conteudo/produto/(\d+)"([^>]*)>(.*?)</a>"#,
        );
        for c in products.captures_iter(block) {
            let id = c[1].to_string();
            // Shelf cards carry `data-type="produto"`; plain menu links do not.
            if !c[2].contains(r#"data-type="produto""#)
                || courses.iter().any(|known| known.id == id)
            {
                continue;
            }
            let title = attr(&c[2], "aria-label")
                .map(clean_text)
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| id.clone());
            let image_url = re(&IMAGE, r#"<img[^>]*\ssrc="([^"]+)""#)
                .captures(&c[3])
                .map(|m| decode_entities(&m[1]));
            courses.push(CourseInfo {
                id,
                title,
                shelf: shelf.clone(),
                image_url,
            });
        }
    }
    courses
}

/// Removes the platform's own `N.` position prefix from a sidebar lesson
/// title (the author's numbering after it is kept).
fn strip_position(title: &str, position: usize) -> String {
    let title = clean_text(title);
    match title.strip_prefix(&format!("{position}.")) {
        Some(rest) if !rest.trim().is_empty() => rest.trim().to_string(),
        _ => title,
    }
}

pub fn parse_outline(html: &str) -> Outline {
    static SECTIONS: OnceLock<Regex> = OnceLock::new();
    static GROUP: OnceLock<Regex> = OnceLock::new();
    static TITLE: OnceLock<Regex> = OnceLock::new();
    static LESSON: OnceLock<Regex> = OnceLock::new();
    let group = re(&GROUP, r#"<div class="section-group"[^>]*>"#);
    let heads: Vec<(usize, &str)> = group
        .find_iter(html)
        .map(|m| (m.start(), m.as_str()))
        .collect();
    let mut product_id = re(
        &SECTIONS,
        r#"class="sections"[^>]*?data-acesso-produto-id="(\d+)""#,
    )
    .captures(html)
    .map(|c| c[1].to_string());
    let mut sections: Vec<Section> = Vec::new();
    for (i, (start, head)) in heads.iter().enumerate() {
        if product_id.is_none() {
            product_id = attr(head, "data-acesso-produto-id").map(str::to_string);
        }
        let Some(id) = attr(head, "data-acesso-secao-id") else {
            continue;
        };
        if sections.iter().any(|s| s.id == id) {
            continue;
        }
        let end = heads.get(i + 1).map_or(html.len(), |(next, _)| *next);
        let block = &html[*start..end];
        let title = re(&TITLE, r#"class="item-titulo">\s*([^<]*)"#)
            .captures(block)
            .map(|c| clean_text(&c[1]))
            .unwrap_or_default();
        let mut lessons: Vec<LessonRef> = Vec::new();
        let items = re(
            &LESSON,
            r#"(?s)<a href="/area/conteudo/aula/(\d+)" class="layer-link">.*?class="item-titulo">\s*([^<]*)"#,
        );
        for c in items.captures_iter(block) {
            if lessons.iter().any(|l| l.id == c[1]) {
                continue;
            }
            lessons.push(LessonRef {
                id: c[1].to_string(),
                title: strip_position(&c[2], lessons.len() + 1),
            });
        }
        sections.push(Section {
            id: id.to_string(),
            title,
            lessons,
        });
    }
    Outline {
        product_id,
        sections,
    }
}

fn embeds_in(html: &str) -> Vec<Embed> {
    static PLAYER: OnceLock<Regex> = OnceLock::new();
    let players = re(
        &PLAYER,
        r#"(?s)class="video video-([a-z0-9_-]+)"\s+data-id="([^"]+)".*?<iframe[^>]*\ssrc=['"]([^'"]+)['"]"#,
    );
    players
        .captures_iter(html)
        .filter_map(|c| {
            let src = decode_entities(&c[3]);
            match &c[1] {
                "pandavideo" => playlist_url(&src).map(|url| (EmbedKind::Panda, url)),
                "youtube" => Some((
                    EmbedKind::YouTube,
                    format!("https://www.youtube.com/watch?v={}", &c[2]),
                )),
                _ => Some((EmbedKind::Other, src)),
            }
            .map(|(kind, url)| Embed { kind, url })
        })
        .collect()
}

pub fn parse_lesson(html: &str) -> LessonPage {
    static TITLE: OnceLock<Regex> = OnceLock::new();
    static ARTICLE: OnceLock<Regex> = OnceLock::new();
    static ATTACH: OnceLock<Regex> = OnceLock::new();
    let article = re(
        &ARTICLE,
        r#"(?s)<article class="aula-conteudo">(.*?)</article>"#,
    )
    .captures(html);
    let article_at = article.as_ref().and_then(|c| c.get(0)).map(|m| m.start());
    let before_article = &html[..article_at.unwrap_or(html.len())];
    let content = article
        .as_ref()
        .map_or("", |c| c.get(1).map_or("", |m| m.as_str()));
    let title = re(
        &TITLE,
        r#"(?s)class="main-title main-title-class[^"]*"[^>]*>(.*?)</div>"#,
    )
    .captures(html)
    .map(|c| clean_text(&c[1]))
    .unwrap_or_default();
    let main_video = embeds_in(before_article).into_iter().next();
    let attachments = re(
        &ATTACH,
        r#"(?s)<a href="(/area/arquivo/anexo\?[^"]+)" class="article-attach[^"]*"[^>]*>.*?<div class="attach-title">(.*?)</div>"#,
    )
    .captures_iter(content)
    .map(|c| Attachment {
        name: clean_text(&c[2]),
        path: decode_entities(&c[1]),
    })
    .collect();
    LessonPage {
        title,
        video_missing: main_video.is_none() && before_article.contains("URL no identificada"),
        main_video,
        content_html: content.trim().to_string(),
        embeds: embeds_in(content),
        attachments,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOGIN: &str = include_str!("../tests/fixtures/login.html");
    const HOME: &str = include_str!("../tests/fixtures/home.html");
    const LESSON: &str = include_str!("../tests/fixtures/lesson_video.html");
    const TEXT: &str = include_str!("../tests/fixtures/lesson_text.html");
    const EMPTY: &str = include_str!("../tests/fixtures/lesson_empty.html");

    #[test]
    fn the_csrf_token_is_read_from_the_login_form() {
        assert_eq!(
            parse_login_token(LOGIN).as_deref(),
            Some("csrf-token-fixture-123")
        );
        assert_eq!(parse_login_token("<html></html>"), None);
    }

    #[test]
    fn a_login_form_is_recognised_and_a_course_page_is_not() {
        assert!(is_login_page(LOGIN));
        assert!(!is_login_page(LESSON));
        assert!(!is_login_page(HOME));
    }

    #[test]
    fn home_lists_each_course_once_with_its_shelf_and_cover() {
        let courses = parse_home(HOME);
        assert_eq!(
            courses,
            vec![
                CourseInfo {
                    id: "546150".into(),
                    title: "Empieza Aqui".into(),
                    shelf: Some("PRIMEROS PASOS".into()),
                    image_url: Some("https://upload.example.test/image?p=A1&w=600&t=crop".into()),
                },
                CourseInfo {
                    id: "574186".into(),
                    title: "Soporte & ayuda".into(),
                    shelf: Some("PRIMEROS PASOS".into()),
                    image_url: Some("https://upload.example.test/image?p=B2".into()),
                },
                CourseInfo {
                    id: "575506".into(),
                    title: "1. INTRODUCCIÓN Y BIENVENIDA".into(),
                    shelf: Some("UGC".into()),
                    image_url: Some("https://upload.example.test/image?p=C3".into()),
                },
            ],
            "menu links outside a vitrine and repeated courses are ignored"
        );
    }

    #[test]
    fn the_outline_has_each_section_once_with_clean_titles() {
        let outline = parse_outline(LESSON);
        assert_eq!(outline.product_id.as_deref(), Some("574182"));
        let sections: Vec<_> = outline
            .sections
            .iter()
            .map(|s| (s.id.as_str(), s.title.as_str(), s.lessons.len()))
            .collect();
        assert_eq!(
            sections,
            [
                ("1647785", "Primeros pasos & bienvenida", 3),
                ("1647786", "Bonus", 1)
            ],
            "the sidebar is rendered twice and must not duplicate sections"
        );
        let titles: Vec<_> = outline.sections[0]
            .lessons
            .iter()
            .map(|l| (l.id.as_str(), l.title.as_str()))
            .collect();
        assert_eq!(
            titles,
            [
                ("8184471", "1 . Bienvenida"),
                ("8184472", "Conociendo las Redes Sociales"),
                ("8184473", "Sin numero propio"),
            ],
            "only the platform's own position prefix is removed"
        );
        assert_eq!(outline.sections[1].lessons[0].title, "Plantillas");
    }

    #[test]
    fn a_page_without_sections_has_an_empty_outline() {
        let outline = parse_outline(TEXT);
        assert!(outline.sections.is_empty());
        assert_eq!(outline.product_id.as_deref(), Some("574182"));
        assert_eq!(parse_outline("<html></html>"), Outline::default());
    }

    #[test]
    fn a_lesson_has_its_panda_video_content_embeds_and_attachments() {
        let page = parse_lesson(LESSON);
        assert_eq!(page.title, "Bienvenida \"Golden\"");
        assert_eq!(
            page.main_video,
            Some(Embed {
                kind: EmbedKind::Panda,
                url: "https://b-vz-a02515ca-43f.tv.pandavideo.com.br/36a394ff-5c24-4b0b-a9c8-4de2be22cf4f/playlist.m3u8".into(),
            })
        );
        assert!(!page.video_missing);
        assert_eq!(
            page.embeds,
            vec![Embed {
                kind: EmbedKind::YouTube,
                url: "https://www.youtube.com/watch?v=u6J7p4TyPIc".into(),
            }]
        );
        assert_eq!(
            page.attachments,
            vec![Attachment {
                name: "Manual Mentorias en vivo.pdf".into(),
                path: "/area/arquivo/anexo?a=01ABC&c=texto&i=5602&n=Manual%20Mentorias.pdf&o=8184471&t=aula&signature=deadbeef".into(),
            }]
        );
        assert!(page.content_html.contains("Hola <b>equipo</b>"));
        assert!(!page.content_html.contains("<article"));
    }

    #[test]
    fn a_text_lesson_flags_the_missing_video() {
        let page = parse_lesson(TEXT);
        assert_eq!(page.main_video, None);
        assert!(page.video_missing);
        assert_eq!(page.title, "Lectura");
        assert!(page.content_html.contains("Solo texto."));
    }

    #[test]
    fn an_empty_lesson_has_no_content_at_all() {
        let page = parse_lesson(EMPTY);
        assert_eq!(page.title, "Vacia");
        assert!(!page.video_missing);
        assert_eq!(page.content_html, "");
        assert!(page.embeds.is_empty() && page.attachments.is_empty());
    }
}
