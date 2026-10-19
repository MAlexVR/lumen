//! Folder naming that matches the previous Go implementation, so existing
//! `Hotmart/<course>/<N. module>/<N. lesson>` folders are recognised, then
//! made Windows-safe by the shared sanitizer.

/// Port of the previous Go `sanitizeFilename`: removes (not replaces) characters
/// Windows forbids, turns `&` into `and`, drops control characters, collapses
/// whitespace, prefixes leading dots with `_` and falls back to `unnamed`.
pub fn clean_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut pending_space = false;
    for c in name.chars() {
        if "<>:\"/\\|?*".contains(c) || (c.is_control() && !c.is_whitespace()) {
            continue;
        }
        if c.is_whitespace() {
            pending_space = true;
            continue;
        }
        if pending_space && !out.is_empty() {
            out.push(' ');
        }
        pending_space = false;
        if c == '&' {
            out.push_str("and");
        } else {
            out.push(c);
        }
    }
    let dots = out.chars().take_while(|c| *c == '.').count();
    let cleaned = format!("{}{}", "_".repeat(dots), &out[dots..]);
    if cleaned.is_empty() {
        "unnamed".to_string()
    } else {
        cleaned
    }
}

/// Course folder name: `<name> - <seller>` (just `<name>` without a seller).
pub fn course_folder_name(name: &str, seller: Option<&str>) -> String {
    let base = clean_name(name);
    match seller.map(str::trim).filter(|s| !s.is_empty()) {
        Some(seller) => format!("{base} - {}", clean_name(seller)),
        None => base,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_name_matches_the_go_sanitizer() {
        assert_eq!(clean_name("A: b/c"), "A bc");
        assert_eq!(clean_name("Q&A <live>"), "QandA live");
        assert_eq!(clean_name("..hidden"), "__hidden");
        assert_eq!(clean_name("a\tb\u{1}c   d"), "a bc d");
        assert_eq!(clean_name("  Módulo 1 - ¿Qué?  "), "Módulo 1 - ¿Qué");
        assert_eq!(clean_name(""), "unnamed");
        assert_eq!(clean_name(" ?? "), "unnamed");
    }

    #[test]
    fn course_folders_are_name_dash_seller_like_the_real_library() {
        assert_eq!(
            course_folder_name("MÉTODO HORIZONTE", Some("Marta Ledesma")),
            "MÉTODO HORIZONTE - Marta Ledesma"
        );
        assert_eq!(
            course_folder_name("Fotografía Nocturna - 15.1", Some("Estudio Lumbre SL")),
            "Fotografía Nocturna - 15.1 - Estudio Lumbre SL"
        );
        assert_eq!(course_folder_name("Solo", None), "Solo");
        assert_eq!(course_folder_name("Solo", Some("  ")), "Solo");
        assert_eq!(course_folder_name("A/B", Some("C*D")), "AB - CD");
    }
}
