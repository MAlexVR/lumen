//! Folder naming helpers: numbered, Windows-safe and MAX_PATH-aware.

use std::path::Path;

use lumen_core::sanitize::{name_budget, sanitize_file_name};

/// How a numbered folder is labelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberStyle {
    /// `02 - Title` (Skool layout).
    Padded,
    /// `2. Title` (Hotmart layout, identical to the previous Go implementation).
    Dotted,
}

impl NumberStyle {
    pub fn label(self, number: usize) -> String {
        match self {
            Self::Padded => format!("{number:02} - "),
            Self::Dotted => format!("{number}. "),
        }
    }
}

/// Length of `path` in UTF-16 units (what Windows MAX_PATH counts).
pub fn path_units(path: &Path) -> usize {
    path.to_string_lossy().encode_utf16().count()
}

/// `<number label><title>` as a folder name that fits below `parent`, leaving
/// `reserve` UTF-16 units for what goes underneath and never exceeding `cap`.
pub fn numbered_folder(
    parent: &Path,
    style: NumberStyle,
    number: usize,
    title: &str,
    reserve: usize,
    cap: usize,
) -> String {
    let budget = name_budget(path_units(parent), reserve).min(cap);
    sanitize_file_name(&format!("{}{title}", style.label(number)), budget)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_core::sanitize::MAX_PATH;

    #[test]
    fn labels_follow_the_two_layouts() {
        assert_eq!(NumberStyle::Padded.label(2), "02 - ");
        assert_eq!(NumberStyle::Padded.label(12), "12 - ");
        assert_eq!(NumberStyle::Dotted.label(2), "2. ");
        assert_eq!(NumberStyle::Dotted.label(12), "12. ");
    }

    #[test]
    fn numbered_folders_are_sanitized() {
        let p = Path::new("/lib");
        assert_eq!(
            numbered_folder(p, NumberStyle::Padded, 3, "Intro: Course?", 30, 60),
            "03 - Intro_ Course_"
        );
        assert_eq!(
            numbered_folder(p, NumberStyle::Dotted, 3, "Intro: Course?", 30, 60),
            "3. Intro_ Course_"
        );
    }

    #[test]
    fn numbered_folders_respect_the_cap_and_max_path() {
        let long = "Título muy largo ".repeat(30);
        let capped = numbered_folder(Path::new("/g"), NumberStyle::Padded, 1, &long, 0, 60);
        assert!(capped.encode_utf16().count() <= 60);
        let parent = Path::new("C:\\Users\\someone\\Videos\\Lumen\\Hotmart\\Course");
        let name = numbered_folder(parent, NumberStyle::Dotted, 1, &long, 150, 255);
        let total = path_units(parent) + 1 + name.encode_utf16().count() + 150;
        assert!(total < MAX_PATH, "{total}");
    }

    #[test]
    fn path_units_counts_utf16() {
        assert_eq!(path_units(Path::new("a\u{1F31E}")), 3);
    }
}
