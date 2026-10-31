// JSON shapes returned by the Tauri commands (lumen-core, camelCase).

export type GroupStatus = "none" | "partial" | "complete";

export type ResourceKind = "file" | "link" | "drm" | "reading" | "video";

export interface Resource {
  name: string;
  path: string;
  url: string | null;
  kind: ResourceKind;
}

export interface Lesson {
  key: string;
  title: string;
  index: number;
  video: string | null;
  description: string | null;
  hasVideo: boolean;
  resources: Resource[];
  /** YouTube: library-relative path of the video's own thumbnail file. */
  thumbnail?: string | null;
}

export interface Section {
  title: string;
  index: number;
  lessons: Lesson[];
}

export interface Course {
  title: string;
  index: number;
  sections: Section[];
}

export type PlatformId = "skool" | "hotmart" | "pluralsight" | "goldenlife" | "youtube";

/** One downloaded group. Skool: a community with courses; Hotmart: one
 * purchased course (exactly one Course whose sections are its modules). */
export interface Group {
  /** Progress identity: Skool group name, Hotmart `hotmart/<folder>`. */
  id: string;
  platform: PlatformId;
  /** Folder name on disk. */
  name: string;
  title: string;
  seller: string | null;
  status: GroupStatus;
  lessonCount: number;
  /** YouTube only: `standalone` is the "Videos sueltos" folder, the rest are playlists. */
  collection?: "playlist" | "standalone";
  courses: Course[];
}

export interface PlatformLibrary {
  id: PlatformId;
  groups: Group[];
}

export interface Library {
  platforms: PlatformLibrary[];
  /** Deprecated alias of the Skool groups. */
  groups: Group[];
}

export interface Position {
  lessonKey: string;
  seconds: number;
}

export interface GroupProgress {
  watched: string[];
  lastLesson: string | null;
  positions: Position[];
}

// Skool commands and download events (src-tauri skool.rs, lumen-core report.rs).

export type DownloadErrorCode =
  | "not_logged_in"
  | "session_expired"
  | "invalid_cookies"
  | "invalid_credentials"
  | "already_running"
  | "no_library"
  | "login_cancelled"
  | "login_timeout"
  | "network"
  | "invalid_token"
  | "invalid_url"
  | "unsupported_url"
  | "unavailable"
  | "ytdlp_missing"
  | "bot_challenge"
  | "diagnose_running"
  | "drm_protected"
  | "internal";

export interface DownloadError {
  code: DownloadErrorCode;
  message: string;
}

export interface PlatformGroup {
  slug: string;
  name: string;
  coverUrl: string | null;
  numCourses: number;
}

export interface LoginOutcome {
  loggedIn: boolean;
  groups: PlatformGroup[];
}

/** Hotmart purchase (hotmart_courses). `folder` is the directory under `Hotmart/`. */
export interface CourseInfo {
  id: string;
  name: string;
  seller: string | null;
  coverUrl: string | null;
  purchaseDate: string | null;
  folder: string;
}

export interface HotmartLoginOutcome {
  loggedIn: boolean;
  courses: CourseInfo[];
}

/** History course returned by Pluralsight; malformed source rows stay visible with an issue. */
export interface PluralsightCourse {
  id: string | null;
  title: string;
  level: string | null;
  imageUrl: string | null;
  folder: string;
  issue: string | null;
}

export interface PluralsightLoginOutcome {
  loggedIn: boolean;
  courses: PluralsightCourse[];
}

/** Course of the Golden Life academy; `shelf` is its section ("UGC", "CREADOR DIGITAL"). */
export interface GoldenlifeCourse {
  id: string | null;
  title: string;
  shelf: string | null;
  imageUrl: string | null;
  folder: string;
  issue: string | null;
}

export interface GoldenlifeLoginOutcome {
  loggedIn: boolean;
  courses: GoldenlifeCourse[];
}

export interface RunningDownload {
  id: string;
  platform: PlatformId;
  /** Skool group slug or Hotmart course id. */
  group: string;
  /** The run holds at its next checkpoint until resumed. */
  paused: boolean;
}

export interface ProgressEvent {
  id: string;
  platform: PlatformId;
  /** Group display name (Hotmart: course name). */
  group: string;
  course: string;
  lesson: string;
  /** Current lesson, 0-100. */
  percent: number;
  completed: number;
  total: number;
}

export type IssueKind = "video" | "video_not_resolved" | "resource" | "module_page" | "empty_lesson" | "course";
export type Severity = "error" | "warning";

export interface Issue {
  lesson: string | null;
  kind: IssueKind;
  severity: Severity;
  host: string | null;
  url: string | null;
  message: string;
}

export interface ReportEvent {
  id: string;
  platform: PlatformId;
  group: string;
  cancelled: boolean;
  totalLessons: number;
  completedLessons: number;
  complete: boolean;
  issues: Issue[];
}

// Library folder (S6.library-root) and deletion (S6.delete) contracts.

export interface RootInfo {
  path: string | null;
  source: "saved" | "detected" | "default";
  exists: boolean;
  writable: boolean;
}

export interface LibrarySummary {
  platforms: ("Skool" | "Hotmart" | "Pluralsight" | "GoldenLife")[];
  hasLibrary: boolean;
}

/** Built from group.id and the Skool course folder name; never a path. */
export interface DeleteTarget {
  platform: PlatformId;
  group: string;
  course?: string;
  /** One lesson, by the key the scan returns (never a path). */
  lesson?: string;
}

export interface DeletePreview {
  name: string;
  platform: PlatformId;
  kind: "group" | "course" | "lesson";
  lessons: number;
  lessonsWithVideo: number;
  bytes: number;
  bytesApprox: boolean;
  path: string;
  runningDownload: boolean;
}

export interface DeleteResult {
  deleted: true;
  movedToTrash: boolean;
  bytesFreed: number;
  bytesApprox: boolean;
  groupRemoved: boolean;
}

export type DeleteErrorCode = "lesson_not_found" | "invalid_target" | "not_found" | "download_running" | "trash_failed" | "internal";

export interface DeleteError {
  code: DeleteErrorCode;
  message: string;
}

// YouTube (S8.B-backend contract).

export type YoutubeQuality = "best1080" | "720" | "audio";

export interface YoutubeItem {
  id: string;
  title: string;
  durationSecs: number | null;
  thumbnailUrl: string | null;
  available: boolean;
  reason: string | null;
}

export interface YoutubePreview {
  kind: "video" | "playlist";
  id: string;
  title: string;
  channel: string;
  thumbnailUrl: string | null;
  durationSecs: number | null;
  /** Folder under `YouTube/` the download will use. */
  folder: string;
  items?: YoutubeItem[];
}

export interface YoutubeRequest {
  url: string;
  /** Video ids; absent = the whole playlist. */
  items?: string[];
  quality: YoutubeQuality;
}

// Udemy diagnostic (S9.udemy-1 contract). Nothing is downloaded.

export type UdemyClassification = Record<"downloadable" | "drm" | "article" | "external" | "quiz" | "unknown", number>;
export type UdemyVerdict = "downloadable" | "partial" | "none" | "unknown";

export interface UdemyEndpoint {
  name: string;
  status: string;
  verified: boolean;
  httpStatus?: number | null;
  note?: string;
}

export interface UdemySummary {
  status: "complete" | "cancelled" | "aborted";
  courses: number;
  lectures: number;
  probed: number;
  classification: UdemyClassification;
  verdicts: Record<UdemyVerdict, number>;
  endpoints: UdemyEndpoint[];
  issues: number;
}

export interface UdemyCourseResult {
  id: number;
  title: string | null;
  lectures: number | null;
  types: Record<"video" | "article" | "quiz" | "other", number>;
  probed: number;
  classification: UdemyClassification;
  verdict: UdemyVerdict;
}

/** The sanitized report file (udemy_diagnostic_last). Only fields the UI reads. */
export interface UdemyReport {
  generatedAt: string;
  status: UdemySummary["status"];
  options: { maxCourses?: number | null; sampleLecturesPerCourse?: number; includeTitles?: boolean };
  endpoints: UdemyEndpoint[];
  courses: UdemyCourseResult[];
  totals?: Partial<UdemySummary>;
  issues: Issue[];
}

export interface UdemyProgress {
  step: "session" | "courses" | "curriculum" | "lectures" | "finish";
  current: number;
  total: number;
  message: string;
}

export interface UdemyDiagnoseOptions {
  maxCourses?: number;
  sampleLecturesPerCourse?: number;
  includeTitles?: boolean;
}
