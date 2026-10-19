use serde::{Deserialize, Serialize};

/// Download state of a group folder, derived from its `.complete` marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupStatus {
    None,
    Partial,
    Complete,
}

/// What a YouTube group is: a downloaded playlist or the folder of single videos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Collection {
    Playlist,
    Standalone,
}

/// Platform folder a group was found under (`<root>/Skool`, `<root>/Hotmart`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LibraryPlatform {
    Skool,
    Hotmart,
    Youtube,
    Pluralsight,
}

impl LibraryPlatform {
    /// Platforms in the order they are listed.
    pub const ALL: [Self; 4] = [Self::Skool, Self::Hotmart, Self::Pluralsight, Self::Youtube];

    /// Folder name under the library root (also the root of every rel path).
    pub fn dir_name(self) -> &'static str {
        match self {
            Self::Skool => "Skool",
            Self::Hotmart => "Hotmart",
            Self::Youtube => "YouTube",
            Self::Pluralsight => "Pluralsight",
        }
    }
}

/// Everything found under `<root>/Skool`, `<root>/Hotmart` and `<root>/YouTube`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    /// One entry per platform folder that exists, Skool first.
    pub platforms: Vec<PlatformLibrary>,
    /// Deprecated alias of the Skool groups, kept until the UI reads `platforms`.
    pub groups: Vec<Group>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformLibrary {
    pub id: LibraryPlatform,
    pub groups: Vec<Group>,
}

impl Library {
    pub fn platform(&self, id: LibraryPlatform) -> Option<&PlatformLibrary> {
        self.platforms.iter().find(|p| p.id == id)
    }
}

/// One downloaded group (Skool) or purchased course (Hotmart).
///
/// `name` is the folder name; `id` is the platform-qualified progress group
/// (equal to `name` for Skool, `hotmart/<folder>` for Hotmart).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: String,
    pub platform: LibraryPlatform,
    pub name: String,
    pub title: String,
    pub seller: Option<String>,
    pub status: GroupStatus,
    pub lesson_count: u64,
    pub courses: Vec<Course>,
    /// YouTube only: a playlist, or the standalone `Videos sueltos` folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection: Option<Collection>,
}

impl Group {
    pub fn count_lessons(courses: &[Course]) -> u64 {
        courses
            .iter()
            .flat_map(|c| &c.sections)
            .map(|s| s.lessons.len() as u64)
            .sum()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Course {
    pub title: String,
    pub index: u64,
    pub sections: Vec<Section>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Section {
    pub title: String,
    pub index: u64,
    pub lessons: Vec<Lesson>,
}

/// One lesson folder. `video` and `description` are root-relative paths with
/// forward slashes. `key` is the stable identity used by progress tracking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lesson {
    pub key: String,
    pub title: String,
    pub index: u64,
    pub video: Option<String>,
    pub description: Option<String>,
    pub has_video: bool,
    /// YouTube only: root-relative path of the lesson's own `thumbnail.<ext>`.
    #[serde(default)]
    pub thumbnail: Option<String>,
    pub resources: Vec<Resource>,
    /// Root-relative folder that holds only this lesson (None for loose
    /// files). Scanner-internal: used to delete one lesson, never serialized.
    #[serde(skip)]
    pub dir: Option<String>,
}

/// What an extra lesson file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResourceKind {
    File,
    /// Small `.txt` file holding an external link (`url` is set).
    Link,
    /// Hotmart DRM attachment (`Materials/drm_<name>`).
    Drm,
    /// Hotmart `Supplementary reading.html`.
    Reading,
    /// A further video of the same lesson.
    Video,
}

/// Extra file in a lesson. `url` is set for small `.txt` link files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resource {
    pub name: String,
    pub path: String,
    pub url: Option<String>,
    pub kind: ResourceKind,
}

/// Stable lesson identity (progress is stored under it, so it must never
/// change): `<ci>-<course>/<si>-<section>/<li>-<lesson>`.
pub fn lesson_key(course: (u64, &str), section: (u64, &str), lesson: (u64, &str)) -> String {
    format!(
        "{}-{}/{}-{}/{}-{}",
        course.0, course.1, section.0, section.1, lesson.0, lesson.1
    )
}

/// Hotmart lesson identity. Platform-qualified so it never collides with a
/// Skool key: `hotmart:<ci>-<course>/<mi>-<module>/<li>-<lesson>`.
pub fn hotmart_lesson_key(course: (u64, &str), module: (u64, &str), lesson: (u64, &str)) -> String {
    format!("hotmart:{}", lesson_key(course, module, lesson))
}

/// YouTube lesson identity (a video of a playlist or of "Videos sueltos"),
/// platform-qualified so it never collides with another platform:
/// `youtube:<ci>-<playlist>/<si>-<playlist>/<li>-<video>`.
pub fn youtube_lesson_key(
    course: (u64, &str),
    section: (u64, &str),
    lesson: (u64, &str),
) -> String {
    format!("youtube:{}", lesson_key(course, section, lesson))
}
