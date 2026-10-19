//! Skool descriptions are `[v2]` + TipTap JSON. Rendered to small, escaped HTML.

use serde_json::Value;

use crate::parse::ResourceLink;

/// True for placeholders such as `[v2][{"type":"paragraph"}]`.
pub fn is_empty_description(desc: &str) -> bool {
    let desc = desc.trim();
    if desc.is_empty() || desc == "false" {
        return true;
    }
    match parse_nodes(desc) {
        Some(nodes) => nodes.iter().all(is_blank_node),
        None => false,
    }
}

/// A paragraph without text, or a node with only blank children.
fn is_blank_node(node: &Value) -> bool {
    match node.get("type").and_then(Value::as_str) {
        Some("paragraph") => content(node).iter().all(is_blank_node),
        Some("text") => node
            .get("text")
            .and_then(Value::as_str)
            .is_none_or(|t| t.trim().is_empty()),
        _ => false,
    }
}

/// Nodes of a `[v2]` description, `None` for plain text or invalid JSON.
fn parse_nodes(desc: &str) -> Option<Vec<Value>> {
    let raw = desc.trim().strip_prefix("[v2]")?;
    serde_json::from_str(raw).ok()
}

fn content(node: &Value) -> &[Value] {
    node.get("content")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// Renders a description to a standalone HTML document. Text and attributes
/// are escaped; only http(s)/mailto links and http(s) images are kept.
pub fn description_to_html(desc: &str) -> String {
    let mut out =
        String::from("<!DOCTYPE html><html><head><meta charset=\"utf-8\"></head><body>\n");
    match parse_nodes(desc) {
        Some(nodes) => nodes.iter().for_each(|n| render(&mut out, n)),
        None => {
            let paragraphs: Vec<String> = desc.split('\n').map(escape).collect();
            out.push_str(&format!("<p>{}</p>", paragraphs.join("</p><p>")));
        }
    }
    out.push_str("</body></html>");
    out
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn is_safe_url(url: &str, allow_mailto: bool) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    lower.starts_with("https://")
        || lower.starts_with("http://")
        || (allow_mailto && lower.starts_with("mailto:"))
}

fn render_children(out: &mut String, node: &Value) {
    content(node).iter().for_each(|c| render(out, c));
}

fn wrap(out: &mut String, tag: &str, node: &Value, trailing_newline: bool) {
    out.push_str(&format!("<{tag}>"));
    render_children(out, node);
    out.push_str(&format!("</{tag}>"));
    if trailing_newline {
        out.push('\n');
    }
}

fn render(out: &mut String, node: &Value) {
    let kind = node.get("type").and_then(Value::as_str).unwrap_or("");
    match kind {
        "paragraph" => wrap(out, "p", node, true),
        "heading" => {
            let level = node
                .pointer("/attrs/level")
                .and_then(Value::as_u64)
                .unwrap_or(2)
                .clamp(1, 6);
            wrap(out, &format!("h{level}"), node, true);
        }
        "text" => render_text(out, node),
        "bulletList" | "unorderedList" => {
            out.push_str("<ul>\n");
            render_children(out, node);
            out.push_str("</ul>\n");
        }
        "orderedList" => {
            out.push_str("<ol>\n");
            render_children(out, node);
            out.push_str("</ol>\n");
        }
        "listItem" => wrap(out, "li", node, true),
        "blockquote" => wrap(out, "blockquote", node, true),
        "hardBreak" => out.push_str("<br>"),
        "codeBlock" => {
            out.push_str("<pre><code>");
            render_children(out, node);
            out.push_str("</code></pre>\n");
        }
        "image" => {
            if let Some(src) = node.pointer("/attrs/src").and_then(Value::as_str) {
                if is_safe_url(src, false) {
                    out.push_str(&format!("<img src=\"{}\">\n", escape(src.trim())));
                }
            }
        }
        _ => render_children(out, node),
    }
}

fn render_text(out: &mut String, node: &Value) {
    let mut html = escape(node.get("text").and_then(Value::as_str).unwrap_or(""));
    let marks = node.get("marks").and_then(Value::as_array);
    for mark in marks.map_or(&[][..], Vec::as_slice) {
        match mark.get("type").and_then(Value::as_str) {
            Some("bold") => html = format!("<b>{html}</b>"),
            Some("italic") => html = format!("<i>{html}</i>"),
            Some("link") => {
                if let Some(href) = mark.pointer("/attrs/href").and_then(Value::as_str) {
                    if is_safe_url(href, true) {
                        html = format!("<a href=\"{}\">{html}</a>", escape(href.trim()));
                    }
                }
            }
            _ => {}
        }
    }
    out.push_str(&html);
}

/// Every hyperlink (text + href) inside a `[v2]` description.
pub fn description_links(desc: &str) -> Vec<ResourceLink> {
    fn walk(node: &Value, out: &mut Vec<ResourceLink>) {
        if node.get("type").and_then(Value::as_str) == Some("text") {
            let text = node.get("text").and_then(Value::as_str).unwrap_or("");
            let marks = node.get("marks").and_then(Value::as_array);
            for mark in marks.map_or(&[][..], Vec::as_slice) {
                if mark.get("type").and_then(Value::as_str) != Some("link") {
                    continue;
                }
                if let Some(href) = mark.pointer("/attrs/href").and_then(Value::as_str) {
                    if !href.is_empty() {
                        out.push(ResourceLink {
                            title: text.to_string(),
                            link: href.to_string(),
                        });
                    }
                }
            }
        }
        content(node).iter().for_each(|c| walk(c, out));
    }
    let mut out = Vec::new();
    for node in parse_nodes(desc).unwrap_or_default() {
        walk(&node, &mut out);
    }
    out
}

/// Removes emoji and decorative symbols (approximation of Unicode So/Sk plus
/// everything outside the BMP) and trims.
pub fn strip_emoji(s: &str) -> String {
    s.chars()
        .filter(|c| !is_symbol(*c))
        .collect::<String>()
        .trim()
        .to_string()
}

fn is_symbol(c: char) -> bool {
    let u = c as u32;
    u > 0xFFFF
        || matches!(
            u,
            0x00A9 | 0x00AE | 0x2122 | 0x200D | 0x20E3
                | 0x2190..=0x21FF
                | 0x2300..=0x23FF
                | 0x25A0..=0x27BF
                | 0x2B00..=0x2BFF
                | 0xFE00..=0xFE0F
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v2(json: &str) -> String {
        format!("[v2]{json}")
    }

    fn body(desc: &str) -> String {
        let html = description_to_html(desc);
        let start = html.find("<body>").map_or(0, |i| i + 6);
        let end = html.rfind("</body>").unwrap_or(html.len());
        html[start..end].trim().to_string()
    }

    #[test]
    fn empty_placeholders_are_detected() {
        for d in [
            "",
            "  ",
            "false",
            r#"[v2][{"type":"paragraph"}]"#,
            r#"[v2][]"#,
            r#"[v2][{"type":"paragraph","content":[]}]"#,
        ] {
            assert!(is_empty_description(d), "{d:?}");
        }
        assert!(!is_empty_description(
            r#"[v2][{"type":"paragraph","content":[{"type":"text","text":"x"}]}]"#
        ));
        assert!(!is_empty_description("plain text"));
        assert!(!is_empty_description(
            r#"[v2][{"type":"image","attrs":{"src":"https://a.test/i.png"}}]"#
        ));
    }

    #[test]
    fn document_wrapper_declares_utf8() {
        let h = description_to_html(&v2(r#"[{"type":"paragraph"}]"#));
        assert!(h.starts_with("<!DOCTYPE html>"));
        assert!(h.contains(r#"<meta charset="utf-8">"#));
        assert!(h.ends_with("</html>"));
    }

    #[test]
    fn text_is_escaped() {
        let d = v2(r#"[{"type":"paragraph","content":[{"type":"text","text":"a <script>&\"'"}]}]"#);
        assert_eq!(body(&d), "<p>a &lt;script&gt;&amp;&quot;&#39;</p>");
    }

    #[test]
    fn marks_nest_and_links_escape_href() {
        let d = v2(r#"[{"type":"paragraph","content":[
              {"type":"text","text":"b","marks":[{"type":"bold"}]},
              {"type":"text","text":"i","marks":[{"type":"italic"}]},
              {"type":"text","text":"l","marks":[{"type":"link","attrs":{"href":"https://a.test/x?a=1&b=\"2\""}}]}
            ]}]"#);
        assert_eq!(
            body(&d),
            "<p><b>b</b><i>i</i><a href=\"https://a.test/x?a=1&amp;b=&quot;2&quot;\">l</a></p>"
        );
    }

    #[test]
    fn unsafe_link_and_image_schemes_are_dropped() {
        let d = v2(r#"[{"type":"paragraph","content":[
              {"type":"text","text":"x","marks":[{"type":"link","attrs":{"href":"javascript:alert(1)"}}]}]},
              {"type":"image","attrs":{"src":"data:text/html,boom"}},
              {"type":"image","attrs":{"src":"https://a.test/i.png"}}]"#);
        let b = body(&d);
        assert!(b.contains("<p>x</p>"));
        assert!(!b.contains("javascript"));
        assert!(!b.contains("data:"));
        assert!(b.contains(r#"<img src="https://a.test/i.png">"#));
    }

    #[test]
    fn headings_lists_quotes_breaks_and_code() {
        let d = v2(
            r#"[{"type":"heading","attrs":{"level":3},"content":[{"type":"text","text":"H"}]},
            {"type":"heading","attrs":{"level":9},"content":[{"type":"text","text":"Z"}]},
            {"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"a"}]}]}]},
            {"type":"orderedList","content":[{"type":"listItem","content":[{"type":"text","text":"b"}]}]},
            {"type":"blockquote","content":[{"type":"paragraph","content":[{"type":"text","text":"q"},{"type":"hardBreak"},{"type":"text","text":"r"}]}]},
            {"type":"codeBlock","content":[{"type":"text","text":"1 < 2"}]},
            {"type":"mystery","content":[{"type":"text","text":"kept"}]}]"#,
        );
        let b = body(&d);
        assert!(b.contains("<h3>H</h3>"));
        assert!(b.contains("<h6>Z</h6>"), "level is clamped to 1..=6: {b}");
        assert!(b.contains("<ul>\n<li><p>a</p>\n</li>\n</ul>"));
        assert!(b.contains("<ol>\n<li>b</li>\n</ol>"));
        assert!(b.contains("<blockquote><p>q<br>r</p>\n</blockquote>"));
        assert!(b.contains("<pre><code>1 &lt; 2</code></pre>"));
        assert!(b.contains("kept"));
    }

    #[test]
    fn plain_and_invalid_descriptions_become_escaped_paragraphs() {
        assert_eq!(body("a<b\nc"), "<p>a&lt;b</p><p>c</p>");
        assert_eq!(body("[v2]{not json"), "<p>[v2]{not json</p>");
    }

    #[test]
    fn links_are_collected_from_nested_content() {
        let d = v2(
            r#"[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[
              {"type":"text","text":"Sheet","marks":[{"type":"link","attrs":{"href":"https://x.test/a.xlsx"}}]}]}]}]},
              {"type":"paragraph","content":[{"type":"text","text":"no link"}]}]"#,
        );
        assert_eq!(
            description_links(&d),
            [ResourceLink {
                title: "Sheet".into(),
                link: "https://x.test/a.xlsx".into()
            }]
        );
        assert!(description_links("plain").is_empty());
        assert!(description_links("[v2]{bad").is_empty());
    }

    #[test]
    fn strip_emoji_keeps_letters_and_digits() {
        assert_eq!(strip_emoji("🚀 Growth Lab ™"), "Growth Lab");
        assert_eq!(
            strip_emoji("Curso ✅ Ñandú 2024 👨\u{200d}💻"),
            "Curso  Ñandú 2024"
        );
        assert_eq!(strip_emoji("Plain"), "Plain");
    }
}
