// Download sessions and progress for every platform. One DownloadCenter
// receives all progress/report events (ids are unique across platforms);
// each platform has a PlatformSession built from a small adapter over its
// commands. Initialized once at startup so events arrive on any screen.
import type { Api } from "../api";
import { finishedALesson, initialDownloads, reduceDownloads, type DownloadAction, type DownloadsState } from "../logic/downloads";
import { toDownloadError } from "../logic/downloadErrors";
import { canonicalUrl, initialSelection, loadQuality, reduceSelection, saveQuality, type SelectionAction } from "../logic/youtube";
import type {
  CourseInfo,
  DownloadError,
  PlatformGroup,
  PlatformId,
  PluralsightCourse,
  GoldenlifeCourse,
  HarbizCourse,
  RunningDownload,
  YoutubePreview,
  YoutubeQuality,
  YoutubeRequest,
} from "../types";
import { app } from "./app.svelte";

export type Session = "unknown" | "disconnected" | "waiting" | "connecting" | "connected";

/** Webview login gives up after this long (backend LOGIN_TIMEOUT). */
export const LOGIN_TIMEOUT_MS = 5 * 60 * 1000;

class DownloadCenter {
  downloads = $state<DownloadsState>(initialDownloads);
  #api: Api | null = null;

  /** Download ids are unique across platforms, so pause and resume need no platform. */
  attach(api: Api): void {
    this.#api = api;
  }

  async setPaused(id: string, paused: boolean): Promise<void> {
    const api = this.#api;
    if (!api) return;
    const ok = await (paused ? api.downloadPause(id) : api.downloadResume(id)).catch(() => false);
    if (ok) this.dispatch({ type: "paused", id, paused });
  }

  dispatch(action: DownloadAction): void {
    this.downloads = reduceDownloads(this.downloads, action);
  }
}

export const center = new DownloadCenter();

/** What a platform session needs from the backend. */
export interface DownloadItemSession {
  startErrors: Record<string, DownloadError>;
  starting: Record<string, boolean>;
  cancelling: Record<string, boolean>;
  start(key: string): Promise<void>;
  cancel(id: string): Promise<void>;
  pause(id: string): Promise<void>;
  resume(id: string): Promise<void>;
  dismiss(id: string): void;
}

export interface SessionAdapter<Item> {
  platform: PlatformId;
  status(): Promise<{ loggedIn: boolean }>;
  /** Absent on platforms that only sign in with credentials. */
  loginWebview?(): Promise<Item[]>;
  logout(): Promise<void>;
  list(): Promise<Item[]>;
  start(key: string): Promise<string>;
  cancel(id: string): Promise<boolean>;
  running(): Promise<RunningDownload[]>;
}

export class PlatformSession<Item> {
  session = $state<Session>("unknown");
  items = $state<Item[]>([]);
  /** Error of the last session action (login, list, logout). */
  sessionError = $state<DownloadError | null>(null);
  /** Errors of the last start attempt, per item key. */
  startErrors = $state<Record<string, DownloadError>>({});
  starting = $state<Record<string, boolean>>({});
  cancelling = $state<Record<string, boolean>>({});
  loginStartedAt = $state<number | null>(null);
  #adapter: SessionAdapter<Item> | null = null;

  get platform(): PlatformId | null {
    return this.#adapter?.platform ?? null;
  }

  async init(adapter: SessionAdapter<Item>): Promise<void> {
    if (this.#adapter) return;
    this.#adapter = adapter;
    try {
      const running = await adapter.running();
      if (running.length) center.dispatch({ type: "attached", running });
      const { loggedIn } = await adapter.status();
      if (loggedIn) await this.#load();
      else this.session = "disconnected";
    } catch (e) {
      this.sessionError = toDownloadError(e);
      this.session = "disconnected";
    }
  }

  async #load(): Promise<void> {
    if (!this.#adapter) return;
    try {
      this.items = await this.#adapter.list();
      this.session = "connected";
      this.sessionError = null;
    } catch (e) {
      const error = toDownloadError(e);
      this.sessionError = error;
      this.session = error.code === "network" || error.code === "internal" ? "connected" : "disconnected";
    }
  }

  async #login(run: () => Promise<Item[]>, waiting: boolean): Promise<boolean> {
    this.session = waiting ? "waiting" : "connecting";
    this.sessionError = null;
    if (waiting) this.loginStartedAt = Date.now();
    try {
      this.items = await run();
      this.session = "connected";
      return true;
    } catch (e) {
      this.sessionError = toDownloadError(e);
      this.session = "disconnected";
      return false;
    } finally {
      this.loginStartedAt = null;
    }
  }

  loginWebview(): Promise<boolean> {
    const a = this.#adapter;
    const login = a?.loginWebview?.bind(a);
    return login ? this.#login(login, true) : Promise.resolve(false);
  }

  /** Manual credentials (Skool cookie header, Hotmart token). */
  loginManual(run: () => Promise<Item[]>): Promise<boolean> {
    return this.#login(run, false);
  }

  async logout(): Promise<void> {
    if (!this.#adapter) return;
    try {
      await this.#adapter.logout();
      this.items = [];
      this.session = "disconnected";
      this.sessionError = null;
    } catch (e) {
      this.sessionError = toDownloadError(e);
    }
  }

  async start(key: string): Promise<void> {
    const a = this.#adapter;
    if (!a || this.starting[key]) return;
    this.starting[key] = true;
    delete this.startErrors[key];
    try {
      const id = await a.start(key);
      center.dispatch({ type: "started", id, platform: a.platform, key });
    } catch (e) {
      const error = toDownloadError(e);
      this.startErrors[key] = error;
      if (error.code === "session_expired" || error.code === "not_logged_in") {
        this.sessionError = error;
        this.session = "disconnected";
      }
    } finally {
      this.starting[key] = false;
    }
  }

  async cancel(id: string): Promise<void> {
    if (!this.#adapter) return;
    // The report event ends the download; the flag stays until then.
    this.cancelling[id] = true;
    const ok = await this.#adapter.cancel(id).catch(() => false);
    if (!ok) this.cancelling[id] = false;
  }

  pause(id: string): Promise<void> {
    return center.setPaused(id, true);
  }

  resume(id: string): Promise<void> {
    return center.setPaused(id, false);
  }

  dismiss(id: string): void {
    center.dispatch({ type: "dismiss", id });
  }
}

export const skool = new PlatformSession<PlatformGroup>();
export const hotmart = new PlatformSession<CourseInfo>();
export const pluralsight = new PlatformSession<PluralsightCourse>();

export function createPluralsightSessionAdapter(api: Api): SessionAdapter<PluralsightCourse> {
  return {
    platform: "pluralsight",
    status: () => api.pluralsightSessionStatus(),
    loginWebview: async () => (await api.pluralsightLoginWebview()).courses,
    logout: () => api.pluralsightLogout(),
    list: () => api.pluralsightCourses(),
    start: (key) => api.pluralsightDownloadStart(key),
    cancel: (id) => api.pluralsightDownloadCancel(id),
    running: () => api.pluralsightDownloadsRunning(),
  };
}

export const goldenlife = new PlatformSession<GoldenlifeCourse>();

export function createGoldenlifeSessionAdapter(api: Api): SessionAdapter<GoldenlifeCourse> {
  return {
    platform: "goldenlife",
    status: () => api.goldenlifeSessionStatus(),
    logout: () => api.goldenlifeLogout(),
    list: () => api.goldenlifeCourses(),
    start: (key) => api.goldenlifeDownloadStart(key),
    cancel: (id) => api.goldenlifeDownloadCancel(id),
    running: () => api.goldenlifeDownloadsRunning(),
  };
}

export const harbiz = new PlatformSession<HarbizCourse>();

export function createHarbizSessionAdapter(api: Api): SessionAdapter<HarbizCourse> {
  return {
    platform: "harbiz",
    status: () => api.harbizSessionStatus(),
    logout: () => api.harbizLogout(),
    list: () => api.harbizCourses(),
    start: (key) => api.harbizDownloadStart(key),
    cancel: (id) => api.harbizDownloadCancel(id),
    running: () => api.harbizDownloadsRunning(),
  };
}

/** YouTube has no session: resolve a link, pick videos and quality, download.
 * Downloads go through the shared centre keyed by the canonical link. */
class YoutubeStore {
  resolving = $state(false);
  preview = $state<YoutubePreview | null>(null);
  error = $state<DownloadError | null>(null);
  selection = $state<string[]>([]);
  quality = $state<YoutubeQuality>("best1080");
  starting = $state(false);
  startError = $state<DownloadError | null>(null);
  /** Display data per canonical link, for entries started in this session. */
  titles = $state<Record<string, string>>({});
  folders = $state<Record<string, string>>({});
  cancelling = $state<Record<string, boolean>>({});
  #requests: Record<string, YoutubeRequest> = {};
  #token = 0;
  #api: Api | null = null;

  async init(api: Api): Promise<void> {
    this.#api = api;
    this.quality = loadQuality(storage());
    const running = await api.youtubeDownloadsRunning().catch(() => []);
    if (running.length) center.dispatch({ type: "attached", running });
  }

  /** Resolves a link; a newer call or cancelResolve makes older results stale. */
  async resolve(url: string): Promise<void> {
    const api = this.#api;
    if (!api || !url.trim()) return;
    const token = ++this.#token;
    this.resolving = true;
    this.error = null;
    this.startError = null;
    try {
      const preview = await api.youtubeResolve(url.trim());
      if (token !== this.#token) return;
      this.preview = preview;
      this.selection = initialSelection(preview.items ?? []);
    } catch (e) {
      if (token !== this.#token) return;
      this.preview = null;
      this.error = toDownloadError(e);
    } finally {
      if (token === this.#token) this.resolving = false;
    }
  }

  cancelResolve(): void {
    this.#token += 1;
    this.resolving = false;
  }

  clear(): void {
    this.cancelResolve();
    this.preview = null;
    this.error = null;
    this.startError = null;
  }

  select(action: SelectionAction): void {
    this.selection = reduceSelection(this.selection, action, this.preview?.items ?? []);
  }

  setQuality(q: YoutubeQuality): void {
    this.quality = q;
    saveQuality(storage(), q);
  }

  async start(): Promise<void> {
    const p = this.preview;
    if (!p || this.starting) return;
    const items = p.items ?? [];
    const allChosen = this.selection.length === items.filter((i) => i.available).length;
    const request: YoutubeRequest = {
      url: canonicalUrl(p),
      quality: this.quality,
      ...(p.kind === "playlist" && !allChosen ? { items: [...this.selection] } : {}),
    };
    this.titles[request.url] = p.title;
    this.folders[request.url] = p.folder;
    await this.#start(request);
  }

  retry(key: string): Promise<void> {
    const request = this.#requests[key];
    return request ? this.#start(request) : Promise.resolve();
  }

  async #start(request: YoutubeRequest): Promise<void> {
    const api = this.#api;
    if (!api) return;
    this.starting = true;
    this.startError = null;
    try {
      const id = await api.youtubeDownloadStart(request);
      this.#requests[request.url] = request;
      center.dispatch({ type: "started", id, platform: "youtube", key: request.url });
    } catch (e) {
      this.startError = toDownloadError(e);
    } finally {
      this.starting = false;
    }
  }

  pause(id: string): Promise<void> {
    return center.setPaused(id, true);
  }

  resume(id: string): Promise<void> {
    return center.setPaused(id, false);
  }

  async cancel(id: string): Promise<void> {
    if (!this.#api) return;
    this.cancelling[id] = true;
    const ok = await this.#api.youtubeDownloadCancel(id).catch(() => false);
    if (!ok) this.cancelling[id] = false;
  }

  dismiss(id: string): void {
    center.dispatch({ type: "dismiss", id });
  }
}

/** localStorage when available; a no-op store otherwise. */
function storage(): { getItem(k: string): string | null; setItem(k: string, v: string): void } {
  try {
    return window.localStorage;
  } catch {
    return { getItem: () => null, setItem: () => {} };
  }
}

export const youtube = new YoutubeStore();

let started = false;

export async function initDownloads(api: Api): Promise<void> {
  if (started) return;
  started = true;
  center.attach(api);
  await api.onDownloadProgress((event) => {
    // A lesson that just finished is already on disk: show it without waiting for the whole queue.
    const landed = finishedALesson(center.downloads, event);
    center.dispatch({ type: "progress", event });
    if (landed) app.rescanSoon();
  });
  await api.onDownloadReport((event) => {
    center.dispatch({ type: "report", event });
    void app.rescan();
  });
  await Promise.all([
    skool.init({
      platform: "skool",
      status: () => api.skoolSessionStatus(),
      loginWebview: async () => (await api.skoolLoginWebview()).groups,
      logout: () => api.skoolLogout(),
      list: () => api.skoolGroups(),
      start: (key) => api.skoolDownloadStart(key),
      cancel: (id) => api.skoolDownloadCancel(id),
      running: () => api.skoolDownloadsRunning(),
    }),
    hotmart.init({
      platform: "hotmart",
      status: () => api.hotmartSessionStatus(),
      loginWebview: async () => (await api.hotmartLoginWebview()).courses,
      logout: () => api.hotmartLogout(),
      list: () => api.hotmartCourses(),
      start: (key) => api.hotmartDownloadStart(key),
      cancel: (id) => api.hotmartDownloadCancel(id),
      running: () => api.hotmartDownloadsRunning(),
    }),
    pluralsight.init(createPluralsightSessionAdapter(api)),
    goldenlife.init(createGoldenlifeSessionAdapter(api)),
    harbiz.init(createHarbizSessionAdapter(api)),
    youtube.init(api),
  ]);
}
