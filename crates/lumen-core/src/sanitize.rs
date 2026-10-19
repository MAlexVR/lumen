//! Windows-safe file name helpers (also valid on Linux and macOS).

/// Windows MAX_PATH including the terminating NUL.
pub const MAX_PATH: usize = 260;
/// Per-component limit on NTFS and most Unix file systems.
pub const MAX_COMPONENT: usize = 255;

/// Sanitize one file or folder name: replaces characters Windows forbids,
/// avoids reserved device names, strips trailing dots/spaces and truncates to
/// `max_len` UTF-16 code units (keeping a short extension when possible).
pub fn sanitize_file_name(name: &str, max_len: usize) -> String {
    let replaced: String = name
        .chars()
        .map(|c| {
            if c.is_control() || ILLEGAL.contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let mut out = trim_edges(&replaced).to_string();
    if is_reserved(&out) {
        out.insert(0, '_');
    }
    let out = truncate_units(&out, max_len.max(1));
    if out.is_empty() {
        "_".to_string()
    } else {
        out
    }
}

const ILLEGAL: &str = "<>:\"/\\|?*";
const RESERVED: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];

fn trim_edges(s: &str) -> &str {
    s.trim_start_matches(' ').trim_end_matches(['.', ' '])
}

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

fn is_reserved(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").trim_end_matches(' ');
    let upper = stem.to_ascii_uppercase();
    if RESERVED.contains(&upper.as_str()) {
        return true;
    }
    let bytes = upper.as_bytes();
    bytes.len() == 4
        && (upper.starts_with("COM") || upper.starts_with("LPT"))
        && (b'1'..=b'9').contains(&bytes[3])
}

fn is_extension(ext: &str, max_len: usize) -> bool {
    let units = utf16_len(ext);
    units > 1
        && units <= 9
        && units < max_len
        && ext[1..].chars().all(|c| c.is_ascii_alphanumeric())
}

/// Keeps at most `max_len` UTF-16 units, preserving a short extension.
fn truncate_units(name: &str, max_len: usize) -> String {
    if utf16_len(name) <= max_len {
        return name.to_string();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 && is_extension(&name[i..], max_len) => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let budget = max_len - utf16_len(ext);
    let mut used = 0;
    let mut kept = String::new();
    for c in stem.chars() {
        used += c.len_utf16();
        if used > budget {
            break;
        }
        kept.push(c);
    }
    let kept = kept.trim_end_matches(['.', ' ']);
    format!("{kept}{ext}")
}

/// Longest component that still fits MAX_PATH when placed under a parent
/// directory whose path is `parent_len` UTF-16 units long, leaving `reserve`
/// units for a suffix such as an extension. Never below 8 or above 255.
pub fn name_budget(parent_len: usize, reserve: usize) -> usize {
    // MAX_PATH counts the NUL, one separator, the parent and the suffix.
    MAX_PATH
        .saturating_sub(1 + parent_len + 1 + reserve)
        .clamp(8, MAX_COMPONENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u16len(s: &str) -> usize {
        s.encode_utf16().count()
    }

    #[test]
    fn replaces_illegal_characters() {
        assert_eq!(
            sanitize_file_name(r#"a<b>c:d"e/f\g|h?i*j"#, 255),
            "a_b_c_d_e_f_g_h_i_j"
        );
    }

    #[test]
    fn replaces_control_characters() {
        assert_eq!(sanitize_file_name("a\u{0}b\u{1f}c", 255), "a_b_c");
    }

    #[test]
    fn strips_trailing_dots_and_spaces() {
        assert_eq!(sanitize_file_name("Lesson 1. . ", 255), "Lesson 1");
    }

    #[test]
    fn strips_leading_spaces() {
        assert_eq!(sanitize_file_name("  intro", 255), "intro");
    }

    #[test]
    fn reserved_names_get_prefix() {
        for n in [
            "CON",
            "nul",
            "Aux",
            "PRN",
            "COM1",
            "lpt9",
            "CON.txt",
            "nul.tar.gz",
        ] {
            let out = sanitize_file_name(n, 255);
            assert!(out.starts_with('_'), "{n} -> {out}");
            assert!(out.ends_with(&n[3..]) || out.len() > n.len());
        }
    }

    #[test]
    fn similar_names_are_not_reserved() {
        assert_eq!(sanitize_file_name("CONSOLE", 255), "CONSOLE");
        assert_eq!(sanitize_file_name("COM10", 255), "COM10");
        assert_eq!(sanitize_file_name("COM0", 255), "COM0");
    }

    #[test]
    fn empty_result_becomes_underscore() {
        assert_eq!(sanitize_file_name("", 255), "_");
        assert_eq!(sanitize_file_name("...", 255), "_");
        assert_eq!(sanitize_file_name("   ", 255), "_");
    }

    #[test]
    fn keeps_unicode_and_emoji() {
        assert_eq!(
            sanitize_file_name("Junio 29 \u{1F31E} \u{00C1}guila", 255),
            "Junio 29 \u{1F31E} \u{00C1}guila"
        );
    }

    #[test]
    fn truncates_by_utf16_units_on_char_boundary() {
        let name = "\u{1F31E}".repeat(50); // 2 units each
        let out = sanitize_file_name(&name, 11);
        assert!(u16len(&out) <= 11, "{}", u16len(&out));
        assert_eq!(out.chars().count(), 5);
    }

    #[test]
    fn truncation_keeps_extension() {
        let out = sanitize_file_name(&format!("{}.mp4", "a".repeat(300)), 40);
        assert_eq!(u16len(&out), 40);
        assert!(out.ends_with(".mp4"));
    }

    #[test]
    fn truncation_retrims_trailing_dots_and_spaces() {
        let out = sanitize_file_name("abcde  . fgh", 7);
        assert_eq!(out, "abcde");
    }

    #[test]
    fn name_budget_respects_max_path() {
        assert_eq!(name_budget(100, 4), 260 - 1 - 100 - 1 - 4);
    }

    #[test]
    fn name_budget_is_clamped() {
        assert_eq!(name_budget(1, 0), 255);
        assert_eq!(name_budget(500, 0), 8);
    }
}
