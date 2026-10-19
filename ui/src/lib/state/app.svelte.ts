// Application state: the scanned library and per-group progress.
import { getApi, mediaUrl, type Api } from "../api";
import { allGroups, groupById } from "../logic/library";
import { flattenGroup, type FlatLesson } from "../logic/progress";
import type { Group, GroupProgress, Library, RootInfo } from "../types";

type Status = "loading" | "ready" | "error";

const ACTIVE_KEY = "lumen.activeGroup";
const EMPTY: GroupProgress = { watched: [], lastLesson: null, positions: [] };

function readActive(): string | null {
  try {
    return localStorage.getItem(ACTIVE_KEY);
  } catch {
    return null;
  }
}

class AppStore {
  status = $state<Status>("loading");
  error = $state("");
  library = $state<Library>({ platforms: [], groups: [] });
  progress = $state<Record<string, GroupProgress>>({});
  root = $state("");
  rootInfo = $state<RootInfo | null>(null);
  mediaBase = $state("");
  mode = $state<Api["mode"]>("mock");
  version = $state("");
  activeGroupName = $state<string | null>(readActive());
  #api: Api | null = null;
  #rescanHooks: (() => void)[] = [];

  /** Runs after every successful rescan (download report, library change, new root). */
  afterRescan(hook: () => void): void {
    this.#rescanHooks.push(hook);
  }

  async init(): Promise<void> {
    try {
      const api = await getApi();
      this.#api = api;
      this.mode = api.mode;
      const [info, base, rootInfo] = await Promise.all([api.appInfo(), api.mediaBase(), api.libraryRoot()]);
      this.version = info.version;
      this.mediaBase = base;
      this.rootInfo = rootInfo;
      this.root = rootInfo.path ?? "";
      // A new library folder or a deletion changes what is on disk: rescan.
      await api.onLibraryRootChanged((e) => {
        this.root = e.path ?? "";
        void api.libraryRoot().then((r) => (this.rootInfo = r));
        void this.rescan();
      });
      await api.onLibraryChanged(() => void this.rescan());
      const library = this.root ? await api.scanLibrary(this.root) : { platforms: [], groups: [] };
      const entries = await Promise.all(
        allGroups(library).map(async (g) => [g.id, await api.progress(g.id)] as const),
      );
      this.progress = Object.fromEntries(entries);
      this.library = library;
      this.status = "ready";
    } catch (e) {
      this.error = String(e);
      this.status = "error";
    }
  }

  /** Re-reads the library and progress, e.g. after a download finished. */
  async rescan(): Promise<void> {
    const api = this.#api;
    if (!api) return;
    if (!this.root) {
      this.library = { platforms: [], groups: [] };
      for (const hook of this.#rescanHooks) hook();
      return;
    }
    try {
      const library = await api.scanLibrary(this.root);
      const entries = await Promise.all(
        allGroups(library).map(async (g) => [g.id, await api.progress(g.id)] as const),
      );
      this.progress = Object.fromEntries(entries);
      this.library = library;
      for (const hook of this.#rescanHooks) hook();
    } catch (e) {
      console.error(e);
    }
  }

  get api(): Api | null {
    return this.#api;
  }

  /** Every group of every platform. */
  get groups(): Group[] {
    return allGroups(this.library);
  }

  /** Group by id (progress identity), never by folder name. */
  group(id: string): Group | undefined {
    return groupById(this.library, id);
  }

  /** Group id of the most recently opened lesson (kept by this app; the
   * backend progress has no timestamps). */
  get recentGroupId(): string | null {
    return this.activeGroupName;
  }

  setActiveGroup(id: string): void {
    this.activeGroupName = id;
    try {
      localStorage.setItem(ACTIVE_KEY, id);
    } catch {
      /* not persisted */
    }
  }

  /** Progress of a group, keyed by `group.id`. */
  progressOf(group: string): GroupProgress {
    return this.progress[group] ?? EMPTY;
  }

  watchedOf(group: string): Set<string> {
    return new Set(this.progressOf(group).watched);
  }

  flat(group: string): FlatLesson[] {
    const g = this.group(group);
    return g ? flattenGroup(g) : [];
  }

  mediaUrl(rel: string): string {
    return mediaUrl(this.mediaBase, rel);
  }

  #ensure(group: string): GroupProgress {
    if (!this.progress[group]) this.progress[group] = { watched: [], lastLesson: null, positions: [] };
    return this.progress[group];
  }

  async setWatched(group: string, key: string, watched: boolean): Promise<void> {
    const p = this.#ensure(group);
    const has = p.watched.includes(key);
    if (watched && !has) p.watched.push(key);
    if (!watched && has) p.watched = p.watched.filter((k) => k !== key);
    await this.#api?.setWatched(group, key, watched).catch((e) => console.error(e));
  }

  async setLast(group: string, key: string): Promise<void> {
    this.#ensure(group).lastLesson = key;
    this.setActiveGroup(group);
    await this.#api?.setLast(group, key).catch((e) => console.error(e));
  }

  async savePosition(group: string, key: string, seconds: number): Promise<void> {
    const s = Math.max(0, Math.floor(seconds));
    const p = this.#ensure(group);
    const existing = p.positions.find((x) => x.lessonKey === key);
    if (existing) existing.seconds = s;
    else p.positions.push({ lessonKey: key, seconds: s });
    await this.#api?.savePosition(group, key, s).catch((e) => console.error(e));
  }
}

export const app = new AppStore();
