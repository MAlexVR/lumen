// Typed access to the Tauri commands. Outside Tauri (browser preview,
// headless screenshots) the same interface is served by an in-memory mock.
import type { InstallStage } from "./logic/updateInstall";
import type { UpdateInfo } from "./logic/updates";
import type { ThumbItem, ThumbReady, ThumbResult } from "./logic/thumbs";
import type {
  CourseInfo,
  DeletePreview,
  DeleteResult,
  DeleteTarget,
  LibrarySummary,
  RootInfo,
  YoutubePreview,
  YoutubeRequest,
  UdemyDiagnoseOptions,
  UdemyProgress,
  UdemyReport,
  UdemySummary,
  GroupProgress,
  HotmartLoginOutcome,
  PluralsightLoginOutcome,
  PluralsightCourse,
  GoldenlifeLoginOutcome,
  GoldenlifeCourse,
  Library,
  LoginOutcome,
  PlatformGroup,
  ProgressEvent,
  ReportEvent,
  RunningDownload,
} from "./types";

export interface AppInfo {
  name: string;
  version: string;
}

export interface Api {
  readonly mode: "tauri" | "mock";
  appInfo(): Promise<AppInfo>;
  mediaBase(): Promise<string>;
  defaultRoot(): Promise<string | null>;
  scanLibrary(root: string): Promise<Library>;
  progress(group: string): Promise<GroupProgress>;
  setWatched(group: string, key: string, watched: boolean): Promise<void>;
  setLast(group: string, key: string): Promise<void>;
  savePosition(group: string, key: string, seconds: number): Promise<void>;
  description(path: string): Promise<string>;
  openPath(path: string): Promise<void>;
  /** Opens the library root (no `rel`) or a library-relative folder in the file manager. */
  openFolder(rel?: string): Promise<void>;
  openUrl(url: string): Promise<void>;

  // Library folder. library_set_root rejects with a plain string (logic/libraryRoot.ts).
  libraryRoot(): Promise<RootInfo>;
  librarySetRoot(path: string): Promise<RootInfo>;
  libraryResetRoot(): Promise<RootInfo>;
  /** Native folder chooser; null when cancelled. Never persists. */
  libraryPickRoot(): Promise<string | null>;
  librarySummary(path: string): Promise<LibrarySummary>;
  onLibraryRootChanged(cb: (e: { path: string | null }) => void): Promise<() => void>;

  // Deleting downloads. Rejections carry `{ code, message }` (logic/deletion.ts).
  libraryDeletePreview(target: DeleteTarget): Promise<DeletePreview>;
  libraryDelete(target: DeleteTarget, permanent: boolean): Promise<DeleteResult>;
  onLibraryChanged(cb: (e: { reason: string; platform: string; group: string; course?: string }) => void): Promise<() => void>;

  // Skool and Hotmart. Rejections carry `{ code, message }` (logic/downloadErrors.ts).
  skoolLoginCookies(text: string): Promise<LoginOutcome>;
  skoolLoginWebview(): Promise<LoginOutcome>;
  skoolSessionStatus(): Promise<{ loggedIn: boolean }>;
  skoolLogout(): Promise<void>;
  skoolGroups(): Promise<PlatformGroup[]>;
  skoolDownloadStart(group: string): Promise<string>;
  skoolDownloadCancel(id: string): Promise<boolean>;
  skoolDownloadsRunning(): Promise<RunningDownload[]>;
  hotmartLoginCredentials(token: string, cookies?: string): Promise<HotmartLoginOutcome>;
  hotmartLoginWebview(): Promise<HotmartLoginOutcome>;
  hotmartSessionStatus(): Promise<{ loggedIn: boolean }>;
  hotmartLogout(): Promise<void>;
  hotmartCourses(): Promise<CourseInfo[]>;
  hotmartDownloadStart(course: string): Promise<string>;
  hotmartDownloadCancel(id: string): Promise<boolean>;
  /** Pause and resume work for a download of any platform: ids are unique across them. */
  downloadPause(id: string): Promise<boolean>;
  downloadResume(id: string): Promise<boolean>;
  hotmartDownloadsRunning(): Promise<RunningDownload[]>;
  // YouTube: no login; resolve a link, then download it.
  youtubeResolve(url: string): Promise<YoutubePreview>;
  youtubeDownloadStart(request: YoutubeRequest): Promise<string>;
  youtubeDownloadCancel(id: string): Promise<boolean>;
  youtubeDownloadsRunning(): Promise<RunningDownload[]>;
  // Udemy: login and a read-only diagnostic (no downloader yet).
  udemyLoginCookies(text: string): Promise<{ loggedIn: boolean; totalCourses: number | null }>;
  udemyLoginWebview(): Promise<{ loggedIn: boolean; totalCourses: number | null }>;
  udemySessionStatus(): Promise<{ loggedIn: boolean }>;
  udemyLogout(): Promise<void>;
  udemyDiagnose(options: UdemyDiagnoseOptions): Promise<{ path: string; summary: UdemySummary }>;
  udemyDiagnoseCancel(): Promise<boolean>;
  udemyDiagnosticLast(): Promise<UdemyReport | null>;
  onUdemyProgress(cb: (e: UdemyProgress) => void): Promise<() => void>;
  pluralsightLoginCredentials(text: string): Promise<PluralsightLoginOutcome>;
  pluralsightLoginWebview(): Promise<PluralsightLoginOutcome>;
  pluralsightSessionStatus(): Promise<{ loggedIn: boolean }>;
  pluralsightLogout(): Promise<void>;
  pluralsightCourses(): Promise<PluralsightCourse[]>;
  pluralsightDownloadStart(course: string): Promise<string>;
  pluralsightDownloadCancel(id: string): Promise<boolean>;
  pluralsightDownloadsRunning(): Promise<RunningDownload[]>;
  goldenlifeLoginCredentials(email: string, password: string): Promise<GoldenlifeLoginOutcome>;
  goldenlifeSessionStatus(): Promise<{ loggedIn: boolean }>;
  goldenlifeLogout(): Promise<void>;
  goldenlifeCourses(): Promise<GoldenlifeCourse[]>;
  goldenlifeDownloadStart(course: string): Promise<string>;
  goldenlifeDownloadCancel(id: string): Promise<boolean>;
  goldenlifeDownloadsRunning(): Promise<RunningDownload[]>;
  // Updates (S12.update): check the releases, settings and the optional read-only token.
  updateCheck(): Promise<UpdateInfo>;
  updateSettings(): Promise<{ includePrereleases: boolean }>;
  updateSetIncludePrereleases(value: boolean): Promise<void>;
  updateTokenStatus(): Promise<{ saved: boolean }>;
  updateTokenSave(token: string): Promise<void>;
  updateTokenClear(): Promise<void>;
  /** Download, verify, install and restart (S14). A successful install ends the app first. */
  updateInstall(): Promise<{ version: string }>;
  updateCancel(): Promise<boolean>;
  onUpdateProgress(cb: (e: { stage: InstallStage; downloaded: number; total: number }) => void): Promise<() => void>;
  // Cached ffmpeg thumbnails (S11.thumbs).
  thumbnailsRequest(items: ThumbItem[], width?: number): Promise<ThumbResult[]>;
  thumbnailsCancel(paths: string[]): Promise<void>;
  thumbnailsStatus(): Promise<{ ffmpegAvailable: boolean; cacheBytes: number; queued: number; running: number }>;
  onThumbnailReady(cb: (e: ThumbReady) => void): Promise<() => void>;
  /** Progress and reports of every platform (match by download id). */
  onDownloadProgress(cb: (e: ProgressEvent) => void): Promise<() => void>;
  onDownloadReport(cb: (e: ReportEvent) => void): Promise<() => void>;
}

/** Media server URL for a library-relative path (each segment encoded). */
export const mediaUrl = (base: string, rel: string): string =>
  `${base}/${rel.split("/").map(encodeURIComponent).join("/")}`;

async function listenTo<T>(event: string, cb: (payload: T) => void): Promise<() => void> {
  const { listen } = await import("@tauri-apps/api/event");
  return listen<T>(event, (e) => cb(e.payload));
}

export type CommandInvoker = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;

/** Pluralsight Tauri contract, separated so invoke names and arguments are testable. */
export function createPluralsightCommands(call: CommandInvoker): Pick<
  Api,
  | "pluralsightLoginCredentials"
  | "pluralsightLoginWebview"
  | "pluralsightSessionStatus"
  | "pluralsightLogout"
  | "pluralsightCourses"
  | "pluralsightDownloadStart"
  | "pluralsightDownloadCancel"
  | "pluralsightDownloadsRunning"
> {
  return {
    pluralsightLoginCredentials: (text) => call("pluralsight_login_credentials", { text }),
    pluralsightLoginWebview: () => call("pluralsight_login_webview"),
    pluralsightSessionStatus: () => call("pluralsight_session_status"),
    pluralsightLogout: () => call("pluralsight_logout"),
    pluralsightCourses: () => call("pluralsight_courses"),
    pluralsightDownloadStart: (course) => call("pluralsight_download_start", { course }),
    pluralsightDownloadCancel: (id) => call("pluralsight_download_cancel", { id }),
    pluralsightDownloadsRunning: () => call("pluralsight_downloads_running"),
  };
}

/** Golden Life Tauri contract: credentials go in once, only the session cookie is kept. */
export function createGoldenlifeCommands(call: CommandInvoker): Pick<
  Api,
  | "goldenlifeLoginCredentials"
  | "goldenlifeSessionStatus"
  | "goldenlifeLogout"
  | "goldenlifeCourses"
  | "goldenlifeDownloadStart"
  | "goldenlifeDownloadCancel"
  | "goldenlifeDownloadsRunning"
> {
  return {
    goldenlifeLoginCredentials: (email, password) => call("goldenlife_login_credentials", { email, password }),
    goldenlifeSessionStatus: () => call("goldenlife_session_status"),
    goldenlifeLogout: () => call("goldenlife_logout"),
    goldenlifeCourses: () => call("goldenlife_courses"),
    goldenlifeDownloadStart: (course) => call("goldenlife_download_start", { course }),
    goldenlifeDownloadCancel: (id) => call("goldenlife_download_cancel", { id }),
    goldenlifeDownloadsRunning: () => call("goldenlife_downloads_running"),
  };
}

function tauriApi(): Api {
  const call = async <T>(cmd: string, args?: Record<string, unknown>): Promise<T> => {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke<T>(cmd, args);
  };
  return {
    mode: "tauri",
    appInfo: () => call("app_info"),
    mediaBase: () => call("media_base"),
    defaultRoot: () => call("library_default_root"),
    scanLibrary: (root) => call("library_scan", { root }),
    progress: (group) => call("progress_group", { group }),
    setWatched: (group, key, watched) => call("progress_set_watched", { group, key, watched }),
    setLast: (group, key) => call("progress_set_last", { group, key }),
    savePosition: (group, key, seconds) =>
      call("progress_save_position", { group, key, seconds: Math.max(0, Math.floor(seconds)) }),
    description: (path) => call("lesson_description", { path }),
    openPath: (path) => call("open_library_path", { path }),
    openFolder: (rel) => call("open_library_folder", { rel: rel ?? null }),
    openUrl: async (url) => {
      const { openUrl } = await import("@tauri-apps/plugin-opener");
      await openUrl(url);
    },
    libraryRoot: () => call("library_root"),
    librarySetRoot: (path) => call("library_set_root", { path }),
    libraryResetRoot: () => call("library_reset_root"),
    libraryPickRoot: () => call("library_pick_root"),
    librarySummary: (path) => call("library_summary", { path }),
    onLibraryRootChanged: (cb) => listenTo<{ path: string | null }>("library://root-changed", cb),
    libraryDeletePreview: (target) => call("library_delete_preview", { target }),
    libraryDelete: (target, permanent) => call("library_delete", { target, permanent }),
    onLibraryChanged: (cb) => listenTo("library://changed", cb),
    skoolLoginCookies: (text) => call("skool_login_cookies", { text }),
    skoolLoginWebview: () => call("skool_login_webview"),
    skoolSessionStatus: () => call("skool_session_status"),
    skoolLogout: () => call("skool_logout"),
    skoolGroups: () => call("skool_groups"),
    skoolDownloadStart: (group) => call("skool_download_start", { group }),
    skoolDownloadCancel: (id) => call("skool_download_cancel", { id }),
    skoolDownloadsRunning: () => call("skool_downloads_running"),
    hotmartLoginCredentials: (token, cookies) =>
      call("hotmart_login_credentials", { token, cookies: cookies?.trim() ? cookies : null }),
    hotmartLoginWebview: () => call("hotmart_login_webview"),
    hotmartSessionStatus: () => call("hotmart_session_status"),
    hotmartLogout: () => call("hotmart_logout"),
    hotmartCourses: () => call("hotmart_courses"),
    hotmartDownloadStart: (course) => call("hotmart_download_start", { course }),
    hotmartDownloadCancel: (id) => call("hotmart_download_cancel", { id }),
    hotmartDownloadsRunning: () => call("hotmart_downloads_running"),
    youtubeResolve: (url) => call("youtube_resolve", { url }),
    youtubeDownloadStart: (request) => call("youtube_download_start", { request }),
    youtubeDownloadCancel: (id) => call("youtube_download_cancel", { id }),
    youtubeDownloadsRunning: () => call("youtube_downloads_running"),
    udemyLoginCookies: (text) => call("udemy_login_cookies", { text }),
    udemyLoginWebview: () => call("udemy_login_webview"),
    udemySessionStatus: () => call("udemy_session_status"),
    udemyLogout: () => call("udemy_logout"),
    udemyDiagnose: (options) => call("udemy_diagnose", { options }),
    udemyDiagnoseCancel: () => call("udemy_diagnose_cancel"),
    udemyDiagnosticLast: () => call("udemy_diagnostic_last"),
    onUdemyProgress: (cb) => listenTo<UdemyProgress>("udemy://diagnose-progress", cb),
    ...createPluralsightCommands(call),
    ...createGoldenlifeCommands(call),
    downloadPause: (id) => call("download_pause", { id }),
    downloadResume: (id) => call("download_resume", { id }),
    updateCheck: () => call("update_check"),
    updateSettings: () => call("update_settings"),
    updateSetIncludePrereleases: (value) => call("update_set_include_prereleases", { value }),
    updateTokenStatus: () => call("update_token_status"),
    updateTokenSave: (token) => call("update_token_save", { token }),
    updateTokenClear: () => call("update_token_clear"),
    updateInstall: () => call("update_install"),
    updateCancel: () => call("update_cancel"),
    onUpdateProgress: (cb) => listenTo<{ stage: InstallStage; downloaded: number; total: number }>("update://progress", cb),
    thumbnailsRequest: (items, width) => call("thumbnails_request", { request: { items, width } }),
    thumbnailsCancel: (paths) => call("thumbnails_cancel", { request: { paths } }),
    thumbnailsStatus: () => call("thumbnails_status"),
    onThumbnailReady: (cb) => listenTo<ThumbReady>("thumbnails://ready", cb),
    onDownloadProgress: (cb) => listenTo<ProgressEvent>("download://progress", cb),
    onDownloadReport: (cb) => listenTo<ReportEvent>("download://report", cb),
  };
}

const inTauri = (): boolean => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

let instance: Api | null = null;

export async function getApi(): Promise<Api> {
  if (!instance) instance = inTauri() ? tauriApi() : (await import("./mock/mockApi")).createMockApi();
  return instance;
}
