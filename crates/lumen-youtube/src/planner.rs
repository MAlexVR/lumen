//! Where YouTube downloads go and how they are named:
//! `<library>/YouTube/<playlist title>/<NN - video title>/<video title>.<ext>`,
//! with single videos under `YouTube/Videos sueltos/`.

use std::path::{Path, PathBuf};

use lumen_core::sanitize::{name_budget, sanitize_file_name};
use lumen_download::layout::{numbered_folder, NumberStyle};
use serde::Deserialize;

pub const PLATFORM_DIR: &str = "YouTube";
/// Folder (inside `YouTube/`) for single videos.
pub const SINGLES_FOLDER: &str = "Videos sueltos";

const GROUP_CAP: usize = 100;
const LESSON_CAP: usize = 80;
/// UTF-16 units kept free below a lesson folder for the media file name.
const FILE_RESERVE: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Quality {
    #[serde(rename = "best1080")]
    Best1080,
    #[serde(rename = "720")]
    P720,
    #[serde(rename = "audio")]
    Audio,
}

impl Quality {
    /// Separate video and audio streams need ffmpeg to be joined.
    pub fn needs_merge(self) -> bool {
        !matches!(self, Self::Audio)
    }
}

/// yt-dlp options for a preset. Without ffmpeg only single-file formats are
/// asked for (lower quality, but they work).
pub fn format_args_for(quality: Quality, ffmpeg: Option<&Path>) -> Vec<String> {
    let height = match quality {
        Quality::Best1080 => Some(1080),
        Quality::P720 => Some(720),
        Quality::Audio => None,
    };
    let mut args = vec!["-f".to_string()];
    match (height, ffmpeg) {
        (Some(h), Some(_)) => {
            args.push(format!("bv*[height<={h}]+ba/b[height<={h}]/b"));
            args.push("--merge-output-format".into());
            args.push("mp4".into());
        }
        (Some(h), None) => args.push(format!("b[height<={h}]/b")),
        (None, _) => args.push("ba[ext=m4a]/ba/b".into()),
    }
    if let Some(ffmpeg) = ffmpeg {
        args.push("--ffmpeg-location".into());
        args.push(ffmpeg.to_string_lossy().into_owned());
    }
    args
}

/// Windows-safe folder name for a playlist title.
pub fn folder_name(title: &str) -> String {
    sanitize_file_name(title, GROUP_CAP)
}

pub fn group_dir(library_root: &Path, title: &str) -> PathBuf {
    library_root.join(PLATFORM_DIR).join(folder_name(title))
}

/// `NN - title` folder that fits under `group_dir` with room for its files.
pub fn lesson_folder(group_dir: &Path, number: usize, title: &str) -> String {
    numbered_folder(
        group_dir,
        NumberStyle::Padded,
        number,
        title,
        FILE_RESERVE,
        LESSON_CAP,
    )
}

/// File name (without extension) of the media inside `lesson_dir`.
pub fn file_stem(lesson_dir: &Path, title: &str) -> String {
    let budget = name_budget(
        lumen_download::layout::path_units(lesson_dir),
        FILE_RESERVE / 2,
    );
    sanitize_file_name(title, budget.clamp(20, 120))
}

/// One more than the highest `NN - ` prefix among the folders of `group_dir`.
pub fn next_number(group_dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(group_dir) else {
        return 1;
    };
    entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let digits: String = name.chars().take_while(char::is_ascii_digit).collect();
            digits.parse::<usize>().ok()
        })
        .max()
        .map_or(1, |m| m + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn group_folders_live_under_youtube_and_are_windows_safe() {
        let root = Path::new("/lib");
        assert_eq!(
            group_dir(root, "Mi lista ñ"),
            Path::new("/lib/YouTube/Mi lista ñ")
        );
        assert_eq!(
            group_dir(root, "A: B/C?"),
            Path::new("/lib/YouTube/A_ B_C_")
        );
        assert_eq!(group_dir(root, "con..."), Path::new("/lib/YouTube/_con"));
        assert_eq!(group_dir(root, ""), Path::new("/lib/YouTube/_"));
        assert_eq!(folder_name("Mi lista ñ"), "Mi lista ñ");
        assert_eq!(SINGLES_FOLDER, "Videos sueltos");
    }

    #[test]
    fn long_group_titles_are_truncated() {
        let long = "Lista larguísima ".repeat(40);
        assert!(folder_name(&long).encode_utf16().count() <= 100);
    }

    #[test]
    fn lesson_folders_are_numbered_and_fit_max_path() {
        let parent = Path::new("/lib/YouTube/L");
        assert_eq!(
            lesson_folder(parent, 3, "Intro: ¿Qué?"),
            "03 - Intro_ ¿Qué_"
        );
        assert_eq!(lesson_folder(parent, 12, "x"), "12 - x");
        let long = "título ".repeat(60);
        let f = lesson_folder(
            Path::new("C:\\Users\\a\\Videos\\Lumen\\YouTube\\Lista"),
            1,
            &long,
        );
        assert!(
            f.encode_utf16().count() <= 80,
            "{}",
            f.encode_utf16().count()
        );
        let stem = file_stem(&Path::new("/lib/YouTube/L").join(&f), &long);
        assert!(stem.encode_utf16().count() < 200);
    }

    #[test]
    fn file_stems_are_sanitized_titles() {
        assert_eq!(
            file_stem(Path::new("/lib/YouTube/L/01 - A"), "Rock & Roll: live"),
            "Rock & Roll_ live"
        );
    }

    #[test]
    fn the_next_free_number_follows_the_highest_existing_one() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(next_number(tmp.path()), 1);
        assert_eq!(next_number(&tmp.path().join("missing")), 1);
        for d in ["01 - A", "02 - B", "10 - C", "notes", ".hidden"] {
            fs::create_dir_all(tmp.path().join(d)).unwrap();
        }
        fs::write(tmp.path().join("99 - file"), "x").unwrap();
        assert_eq!(next_number(tmp.path()), 11);
    }

    #[test]
    fn quality_presets_map_to_format_arguments() {
        let ff = Path::new("/opt/ffmpeg");
        let join = |q, f| format_args_for(q, f).join(" ");
        assert_eq!(
            join(Quality::Best1080, Some(ff)),
            "-f bv*[height<=1080]+ba/b[height<=1080]/b --merge-output-format mp4 --ffmpeg-location /opt/ffmpeg"
        );
        assert_eq!(
            join(Quality::P720, Some(ff)),
            "-f bv*[height<=720]+ba/b[height<=720]/b --merge-output-format mp4 --ffmpeg-location /opt/ffmpeg"
        );
        assert_eq!(join(Quality::Best1080, None), "-f b[height<=1080]/b");
        assert_eq!(join(Quality::P720, None), "-f b[height<=720]/b");
        assert_eq!(join(Quality::Audio, None), "-f ba[ext=m4a]/ba/b");
        assert_eq!(
            join(Quality::Audio, Some(ff)),
            "-f ba[ext=m4a]/ba/b --ffmpeg-location /opt/ffmpeg"
        );
    }

    #[test]
    fn quality_deserializes_from_the_ui_values() {
        let q: Vec<Quality> = serde_json::from_str(r#"["best1080","720","audio"]"#).unwrap();
        assert_eq!(q, [Quality::Best1080, Quality::P720, Quality::Audio]);
        assert!(serde_json::from_str::<Quality>(r#""4k""#).is_err());
        assert!(Quality::Best1080.needs_merge() && Quality::P720.needs_merge());
        assert!(!Quality::Audio.needs_merge());
    }
}
