//! Pure parsers over Hotmart JSON and player HTML. No I/O: the shapes are the
//! ones the previous Go implementation expected.

use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

use crate::error::HotmartError;

/// One entry of `check_token.resources`: maps a product to its club subdomain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClubResource {
    pub product_id: String,
    pub subdomain: String,
}

/// A purchased product that has a members area (club) we can download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotmartCourse {
    /// Product id (stable; also the `x-product-id` header).
    pub id: String,
    pub name: String,
    /// Club slug/subdomain (the `slug` header and referer host).
    pub slug: String,
    pub seller: Option<String>,
    pub cover_url: Option<String>,
    /// `YYYY-MM-DD` when the purchase carried a recognisable date.
    pub purchase_date: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Purchases {
    pub courses: Vec<HotmartCourse>,
    /// Purchases without a members area (or without a product id): not downloadable.
    pub skipped: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavPage {
    pub hash: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavModule {
    pub name: String,
    pub pages: Vec<NavPage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Navigation {
    pub modules: Vec<NavModule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentRef {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Video,
    Audio,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaSrc {
    pub kind: MediaKind,
    pub url: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    pub url: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LessonData {
    pub attachments: Vec<AttachmentRef>,
    pub has_player_media: bool,
    pub medias: Vec<MediaSrc>,
    /// Lesson HTML (description and embedded iframes).
    pub content: String,
    pub readings: Vec<Reading>,
}

/// How an attachment is fetched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachmentLink {
    Direct {
        url: String,
    },
    /// DRM-protected: GET `lambda_url` with the `token` header; its body is the file URL.
    Drm {
        lambda_url: String,
        token: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaAsset {
    pub url: String,
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IframeKind {
    Vimeo,
    Panda,
    YouTube,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Iframe {
    pub kind: IframeKind,
    pub url: String,
}

/// String value of `key`: strings as-is, numbers formatted, anything else empty.
fn text(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 && n.as_i64().is_none() && n.as_u64().is_none() => {
                format!("{f:.0}")
            }
            _ => n.to_string(),
        },
        _ => String::new(),
    }
}

fn array<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn next_data_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // The pattern is a constant; compilation cannot fail.
        Regex::new(r#"(?s)id="__NEXT_DATA__"[^>]*>(.*?)</script>"#)
            .unwrap_or_else(|_| unreachable!("static regex"))
    })
}

fn iframe_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // The pattern is a constant; compilation cannot fail.
        Regex::new(r#"(?is)<iframe[^>]+src=["']([^"']+)["'][^>]*>"#)
            .unwrap_or_else(|_| unreachable!("static regex"))
    })
}

fn date_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // The pattern is a constant; compilation cannot fail.
        Regex::new(r"\d{4}-\d{2}-\d{2}").unwrap_or_else(|_| unreachable!("static regex"))
    })
}

/// Keywords that mark an iframe as video (allow-list from the previous Go implementation, matched on the whole tag).
const IFRAME_ALLOW: [&str; 6] = [
    "vimeo",
    "youtu",
    "pandavideo",
    "liquid",
    "wistia",
    "videodelivery",
];

/// HTML attribute values are entity-encoded; `//host/x` means https.
fn normalize_src(src: &str) -> String {
    let src = src.trim().replace("&amp;", "&");
    match src.strip_prefix("//") {
        Some(rest) => format!("https://{rest}"),
        None => src,
    }
}

/// Keys that may carry the purchase date, most specific first.
const DATE_KEYS: [&str; 6] = [
    "purchaseDate",
    "purchase_date",
    "orderDate",
    "createdAt",
    "creationDate",
    "date",
];

/// `YYYY-MM-DD` from an ISO-like string or an epoch (seconds or milliseconds).
fn purchase_date(purchase: &Value) -> Option<String> {
    DATE_KEYS.iter().find_map(|key| match purchase.get(key)? {
        Value::String(s) => date_re().find(s).map(|m| m.as_str().to_string()),
        Value::Number(n) => {
            let secs = n.as_i64()?;
            let secs = if secs.abs() >= 100_000_000_000 {
                secs / 1000
            } else {
                secs
            };
            Some(iso_date(secs.div_euclid(86_400)))
        }
        _ => None,
    })
}

/// Civil date of a day count since 1970-01-01 (Howard Hinnant's algorithm).
fn iso_date(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

fn course_from_purchase(purchase: &Value, clubs: &[ClubResource]) -> Option<HotmartCourse> {
    let product = purchase.get("product").filter(|p| p.is_object())?;
    let id = text(product, "id");
    if id.is_empty() {
        return None;
    }
    let mut slug = product
        .get("hotmartClub")
        .map(|c| text(c, "slug"))
        .unwrap_or_default();
    let subdomain = clubs
        .iter()
        .find(|c| c.product_id == id)
        .map(|c| c.subdomain.clone())
        .unwrap_or_default();
    if slug.is_empty() {
        slug = subdomain;
    }
    if slug.is_empty() {
        return None;
    }
    let name = collapse(&text(product, "name"));
    let seller = product
        .get("seller")
        .map(|s| collapse(&text(s, "name")))
        .filter(|s| !s.is_empty());
    Some(HotmartCourse {
        id,
        name: if name.is_empty() {
            "Unnamed Course".into()
        } else {
            name
        },
        slug,
        seller,
        cover_url: Some(text(product, "picture")).filter(|p| !p.is_empty()),
        purchase_date: purchase_date(purchase).or_else(|| purchase_date(product)),
    })
}

/// `resources[].resource.{productId,subdomain}` of the `check_token` answer.
pub fn parse_check_token(body: &Value) -> Vec<ClubResource> {
    let Some(resources) = body.get("resources").and_then(Value::as_array) else {
        return Vec::new();
    };
    resources
        .iter()
        .filter_map(|entry| {
            let r = entry.get("resource")?;
            Some(ClubResource {
                product_id: text(r, "productId"),
                subdomain: text(r, "subdomain"),
            })
        })
        .collect()
}

/// `data[].product` of the purchases answer, matched with the club resources.
pub fn parse_purchases(body: &Value, clubs: &[ClubResource]) -> Result<Purchases, HotmartError> {
    let data = body
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| HotmartError::Shape("purchases answer has no data array".into()))?;
    let mut out = Purchases::default();
    for purchase in data {
        match course_from_purchase(purchase, clubs) {
            Some(course) => out.courses.push(course),
            None => out.skipped += 1,
        }
    }
    Ok(out)
}

/// `product.membership.registerAddress` of the product detail answer.
pub fn parse_product_referer(body: &Value) -> Option<String> {
    let address = text(body.get("product")?.get("membership")?, "registerAddress");
    (!address.is_empty()).then_some(address)
}

pub fn parse_navigation(body: &Value) -> Result<Navigation, HotmartError> {
    let modules = body
        .get("modules")
        .and_then(Value::as_array)
        .filter(|m| !m.is_empty());
    let Some(modules) = modules else {
        return Err(match text(body, "message") {
            m if !m.is_empty() => HotmartError::Api(m),
            _ => HotmartError::Shape("course has no modules".into()),
        });
    };
    Ok(Navigation {
        modules: modules
            .iter()
            .map(|m| NavModule {
                name: text(m, "name"),
                pages: array(m, "pages")
                    .iter()
                    .map(|p| NavPage {
                        hash: text(p, "hash"),
                        name: text(p, "name"),
                    })
                    .collect(),
            })
            .collect(),
    })
}

/// A lesson answer; a non-empty `message` is an API error.
pub fn parse_lesson(body: &Value) -> Result<LessonData, HotmartError> {
    if !body.is_object() {
        return Err(HotmartError::Shape("lesson answer is not an object".into()));
    }
    let message = text(body, "message");
    if !message.is_empty() {
        return Err(HotmartError::Api(message));
    }
    Ok(LessonData {
        attachments: array(body, "attachments")
            .iter()
            .filter_map(|a| {
                let id = text(a, "fileMembershipId");
                (!id.is_empty()).then(|| AttachmentRef {
                    id,
                    name: text(a, "fileName"),
                })
            })
            .collect(),
        has_player_media: body
            .get("hasPlayerMedia")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        medias: array(body, "mediasSrc")
            .iter()
            .map(|m| {
                let kind = text(m, "mediaType").to_ascii_uppercase();
                MediaSrc {
                    kind: if kind.contains("VIDEO") {
                        MediaKind::Video
                    } else if kind.contains("AUDIO") {
                        MediaKind::Audio
                    } else {
                        MediaKind::Other
                    },
                    url: text(m, "mediaSrcUrl"),
                    name: text(m, "mediaName"),
                }
            })
            .collect(),
        content: text(body, "content"),
        readings: array(body, "complementaryReadings")
            .iter()
            .filter_map(|r| {
                let url = text(r, "articleUrl");
                (!url.is_empty()).then(|| Reading {
                    url,
                    name: text(r, "articleName"),
                })
            })
            .collect(),
    })
}

/// Decides between the direct and the DRM flow from the raw answer body.
pub fn parse_attachment_link(body: &str) -> Result<AttachmentLink, HotmartError> {
    let json: Value = serde_json::from_str(body).map_err(|e| HotmartError::Json(e.to_string()))?;
    if body.contains("drm-protection") {
        let lambda_url = text(&json, "lambdaUrl");
        if lambda_url.is_empty() {
            return Err(HotmartError::Shape(
                "DRM attachment has no lambdaUrl".into(),
            ));
        }
        return Ok(AttachmentLink::Drm {
            lambda_url,
            token: text(&json, "token"),
        });
    }
    let url = text(&json, "directDownloadUrl");
    if url.is_empty() {
        return Err(HotmartError::Shape(
            "attachment has no directDownloadUrl".into(),
        ));
    }
    Ok(AttachmentLink::Direct { url })
}

/// Media assets from the `__NEXT_DATA__` of a native player page.
pub fn parse_media_assets(html: &str) -> Result<Vec<MediaAsset>, HotmartError> {
    let raw = next_data_re()
        .captures(html)
        .and_then(|c| c.get(1))
        .ok_or_else(|| HotmartError::Shape("player page has no __NEXT_DATA__".into()))?
        .as_str();
    let data: Value = serde_json::from_str(raw).map_err(|e| HotmartError::Json(e.to_string()))?;
    let assets = data
        .pointer("/props/pageProps/applicationData/mediaAssets")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    Ok(assets
        .iter()
        .filter_map(|a| {
            let url = text(a, "url");
            (!url.is_empty()).then(|| MediaAsset {
                url,
                content_type: Some(text(a, "content_type")).filter(|c| !c.is_empty()),
            })
        })
        .collect())
}

/// Video iframes from a known provider found in lesson HTML.
pub fn extract_iframes(html: &str) -> Vec<Iframe> {
    iframe_re()
        .captures_iter(html)
        .filter_map(|c| {
            let tag = c.get(0)?.as_str().to_ascii_lowercase();
            if !IFRAME_ALLOW.iter().any(|kw| tag.contains(kw)) {
                return None;
            }
            let url = normalize_src(c.get(1)?.as_str());
            let lower = url.to_ascii_lowercase();
            let kind = if lower.contains("vimeo.com") {
                IframeKind::Vimeo
            } else if lower.contains("pandavideo") {
                IframeKind::Panda
            } else if lower.contains("youtu") {
                IframeKind::YouTube
            } else {
                IframeKind::Other
            };
            Some(Iframe { kind, url })
        })
        .collect()
}

/// PandaVideo player URL to its direct HLS playlist.
pub fn panda_to_m3u8(url: &str) -> String {
    let mut m = url.to_string();
    if let Some(i) = m.find("player-vz-") {
        m = format!("{}b-vz-{}", &m[..i], &m[i + "player-vz-".len()..]);
    }
    if let Some(i) = m.find("/embed/?v=") {
        m = format!("{}/{}", &m[..i], &m[i + "/embed/?v=".len()..]);
    }
    format!("{m}/playlist.m3u8")
}

/// Base domain of a PandaVideo URL, used as the Referer for its playlist.
pub fn panda_referer(url: &str) -> String {
    match url.find("com.br") {
        Some(i) => url[..i + "com.br".len()].to_string(),
        None => url.to_string(),
    }
}

/// `Supplementary reading.html` body: one escaped link per reading.
pub fn readings_html(readings: &[Reading]) -> String {
    readings
        .iter()
        .map(|r| {
            let label = if r.name.trim().is_empty() {
                &r.url
            } else {
                &r.name
            };
            format!(
                "<a href=\"{}\">{}</a><br>\n",
                escape_html(&r.url),
                escape_html(label)
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    const CHECK: &str = include_str!("../tests/fixtures/check_token.json");
    const PURCHASES: &str = include_str!("../tests/fixtures/purchases.json");
    const NAV: &str = include_str!("../tests/fixtures/navigation.json");
    const LESSON: &str = include_str!("../tests/fixtures/lesson_full.json");
    const CONTENT: &str = include_str!("../tests/fixtures/lesson_content.html");
    const PLAYER: &str = include_str!("../tests/fixtures/player.html");

    #[test]
    fn check_token_yields_product_to_subdomain_pairs() {
        let r = parse_check_token(&json(CHECK));
        assert_eq!(
            r,
            [
                ClubResource {
                    product_id: "1001".into(),
                    subdomain: "horizonte-club".into()
                },
                ClubResource {
                    product_id: "1002".into(),
                    subdomain: "flexibilidad".into()
                },
                ClubResource {
                    product_id: "1004".into(),
                    subdomain: String::new()
                },
            ]
        );
        assert!(parse_check_token(&json("{}")).is_empty());
        assert!(parse_check_token(&json("[]")).is_empty());
    }

    #[test]
    fn purchases_resolve_slug_name_seller_cover_and_date() {
        let clubs = parse_check_token(&json(CHECK));
        let p = parse_purchases(&json(PURCHASES), &clubs).unwrap();
        let ids: Vec<_> = p.courses.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["1001", "1002", "1003", "1005"]);
        assert_eq!(p.skipped, 3, "no club, no product, no id");
        let c = &p.courses[0];
        assert_eq!(c.name, "METODO HORIZONTE");
        assert_eq!(c.slug, "horizonte-club");
        assert_eq!(c.seller.as_deref(), Some("Marta Ledesma"));
        assert_eq!(
            c.cover_url.as_deref(),
            Some("https://cdn.example.test/horizonte.jpg")
        );
        assert_eq!(c.purchase_date.as_deref(), Some("2024-03-09"));
        let flexibilidad = &p.courses[1];
        assert_eq!(
            flexibilidad.slug, "flexibilidad",
            "slug comes from check_token when the product has none"
        );
        assert_eq!(
            flexibilidad.purchase_date.as_deref(),
            Some("2024-03-09"),
            "epoch milliseconds"
        );
        assert_eq!(flexibilidad.cover_url, None);
        assert_eq!(p.courses[2].slug, "own-slug");
        assert_eq!(p.courses[2].seller, None);
        assert_eq!(p.courses[3].name, "Unnamed Course");
    }

    #[test]
    fn purchases_without_a_data_array_are_a_shape_error() {
        assert!(matches!(
            parse_purchases(&json(r#"{"message":"x"}"#), &[]),
            Err(HotmartError::Shape(_))
        ));
        assert!(parse_purchases(&json(r#"{"data":[]}"#), &[])
            .unwrap()
            .courses
            .is_empty());
    }

    #[test]
    fn the_product_referer_is_the_registration_address() {
        assert_eq!(
            parse_product_referer(&json(include_str!("../tests/fixtures/product_detail.json")))
                .as_deref(),
            Some("https://area.example.test/horizonte/")
        );
        assert_eq!(parse_product_referer(&json("{}")), None);
        assert_eq!(
            parse_product_referer(&json(
                r#"{"product":{"membership":{"registerAddress":""}}}"#
            )),
            None
        );
    }

    #[test]
    fn navigation_keeps_every_module_and_page_in_order() {
        let n = parse_navigation(&json(NAV)).unwrap();
        assert_eq!(n.modules.len(), 3);
        assert_eq!(n.modules[0].name, "¡Bienvenidos!");
        assert_eq!(
            n.modules[0].pages,
            [
                NavPage {
                    hash: "h-welcome".into(),
                    name: "Welcome video".into()
                },
                NavPage {
                    hash: "h-guide".into(),
                    name: String::new()
                },
            ]
        );
        assert_eq!(
            n.modules[1].pages[0].hash, "",
            "pages without a hash are kept so they get reported"
        );
        assert!(n.modules[2].pages.is_empty());
    }

    #[test]
    fn navigation_without_modules_is_a_shape_error() {
        assert!(matches!(
            parse_navigation(&json("{}")),
            Err(HotmartError::Shape(_))
        ));
        assert!(matches!(
            parse_navigation(&json(r#"{"modules":[]}"#)),
            Err(HotmartError::Shape(m)) if m.contains("no modules")
        ));
        assert!(matches!(
            parse_navigation(&json(r#"{"message":"Forbidden course"}"#)),
            Err(HotmartError::Api(m)) if m == "Forbidden course"
        ));
    }

    #[test]
    fn lesson_parses_attachments_media_content_and_readings() {
        let l = parse_lesson(&json(LESSON)).unwrap();
        assert_eq!(
            l.attachments,
            [AttachmentRef {
                id: "att-1".into(),
                name: "Plan: 2021 (4).pdf".into()
            }],
            "attachments without an id cannot be fetched"
        );
        assert!(l.has_player_media);
        assert_eq!(l.medias.len(), 3);
        assert_eq!(l.medias[0].kind, MediaKind::Video);
        assert_eq!(
            l.medias[0].url,
            "https://cf-embed.play.hotmart.com/embed/abc"
        );
        assert_eq!(l.medias[1].kind, MediaKind::Audio);
        assert_eq!(l.medias[1].name, "Podcast: ep 1.mp3");
        assert_eq!(l.medias[2].kind, MediaKind::Other);
        assert!(l.content.starts_with("<p>Hello</p>"));
        assert_eq!(l.readings.len(), 2);
        assert_eq!(l.readings[0].url, "https://read.test/a?x=1&y=2");
    }

    #[test]
    fn lesson_defaults_and_api_errors() {
        let l = parse_lesson(&json("{}")).unwrap();
        assert_eq!(l, LessonData::default());
        assert!(matches!(
            parse_lesson(&json(include_str!("../tests/fixtures/lesson_error.json"))),
            Err(HotmartError::Api(m)) if m.contains("not available")
        ));
        assert!(matches!(
            parse_lesson(&json("[]")),
            Err(HotmartError::Shape(_))
        ));
    }

    #[test]
    fn attachment_links_pick_the_direct_or_the_drm_flow() {
        assert_eq!(
            parse_attachment_link(include_str!("../tests/fixtures/attachment_direct.json"))
                .unwrap(),
            AttachmentLink::Direct {
                url: "https://files.example.test/plan.pdf?sig=SECRET".into()
            }
        );
        assert_eq!(
            parse_attachment_link(include_str!("../tests/fixtures/attachment_drm.json")).unwrap(),
            AttachmentLink::Drm {
                lambda_url: "https://lambda.example.test/drm".into(),
                token: "DRMTOKEN".into()
            }
        );
        assert!(matches!(
            parse_attachment_link("{}"),
            Err(HotmartError::Shape(_))
        ));
        assert!(matches!(
            parse_attachment_link("nope"),
            Err(HotmartError::Json(_))
        ));
        assert!(matches!(
            parse_attachment_link(r#"{"type":"drm-protection"}"#),
            Err(HotmartError::Shape(_))
        ));
    }

    #[test]
    fn player_pages_expose_their_media_assets() {
        let a = parse_media_assets(PLAYER).unwrap();
        assert_eq!(a.len(), 2);
        assert_eq!(
            a[0].url,
            "https://cdn.example.test/video/master.m3u8?t=SECRET"
        );
        assert_eq!(a[1].content_type.as_deref(), Some("audio/mpeg"));
        assert!(matches!(
            parse_media_assets("<html></html>"),
            Err(HotmartError::Shape(_))
        ));
        assert!(matches!(
            parse_media_assets(r#"<script id="__NEXT_DATA__">{bad</script>"#),
            Err(HotmartError::Json(_))
        ));
        assert!(
            parse_media_assets(r#"<script id="__NEXT_DATA__">{"props":{}}</script>"#)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn iframes_from_known_providers_are_classified_and_normalised() {
        let f = extract_iframes(CONTENT);
        let got: Vec<(IframeKind, &str)> = f.iter().map(|i| (i.kind, i.url.as_str())).collect();
        assert_eq!(
            got,
            [
                (IframeKind::Vimeo, "https://player.vimeo.com/video/111?h=aa&badge=0"),
                (IframeKind::Panda, "https://player-vz-abc123.tv.pandavideo.com.br/embed/?v=uuid-1"),
                (IframeKind::YouTube, "https://www.youtube.com/embed/YT_ID?rel=0"),
                (IframeKind::Other, "https://fast.wistia.net/embed/iframe/wid"),
                (IframeKind::Other, "https://ads.example.test/frame"),
            ],
            "unknown hosts are ignored, but a known keyword anywhere in the tag counts (rule kept from the previous Go implementation)"
        );
        assert!(extract_iframes("<p>none</p>").is_empty());
    }

    #[test]
    fn panda_urls_become_playlists_with_their_base_referer() {
        let url = "https://player-vz-abc123.tv.pandavideo.com.br/embed/?v=uuid-1";
        assert_eq!(
            panda_to_m3u8(url),
            "https://b-vz-abc123.tv.pandavideo.com.br/uuid-1/playlist.m3u8"
        );
        assert_eq!(
            panda_referer(url),
            "https://player-vz-abc123.tv.pandavideo.com.br"
        );
        assert_eq!(
            panda_referer("https://other.test/x"),
            "https://other.test/x"
        );
    }

    #[test]
    fn readings_html_escapes_names_and_urls() {
        let r = [
            Reading {
                url: "https://read.test/a?x=1&y=2".into(),
                name: "Read <me> & more".into(),
            },
            Reading {
                url: "https://read.test/b".into(),
                name: "Second".into(),
            },
        ];
        assert_eq!(
            readings_html(&r),
            "<a href=\"https://read.test/a?x=1&amp;y=2\">Read &lt;me&gt; &amp; more</a><br>\n\
             <a href=\"https://read.test/b\">Second</a><br>\n"
        );
        assert_eq!(readings_html(&[]), "");
    }
}
