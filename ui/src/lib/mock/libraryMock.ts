// Library folder and deletion parts of the preview mock. Presets:
// `?pick=existing|empty|error|cancel` for the folder chooser and
// `?delete=running|trashfail|notfound` for deletion.
import type { Api } from "../api";
import { allGroups } from "../logic/library";
import type { DeletePreview, Library, LibrarySummary, RootInfo } from "../types";

type LibraryApi = Pick<
  Api,
  | "libraryRoot"
  | "librarySetRoot"
  | "libraryResetRoot"
  | "libraryPickRoot"
  | "librarySummary"
  | "onLibraryRootChanged"
  | "libraryDeletePreview"
  | "libraryDelete"
  | "onLibraryChanged"
>;

const DEFAULT_ROOT = "/home/usuario/Lumen";
const PICKS: Record<string, string> = {
  existing: "/home/usuario/Vídeos/Cursos descargados/Biblioteca principal",
  empty: "/home/usuario/Vídeos/Lumen nueva",
  error: "/home/usuario/Documentos/solo-lectura",
};
const SUMMARIES: Record<string, LibrarySummary> = {
  [PICKS.existing]: { platforms: ["Skool", "Hotmart", "Pluralsight"], hasLibrary: true },
  [PICKS.empty]: { platforms: [], hasLibrary: false },
  [PICKS.error]: { platforms: [], hasLibrary: false },
};

/** `library` is the mock's live library: deletions remove entries from it. */
export function createLibraryMock(params: URLSearchParams, library: Library): LibraryApi {
  let root: RootInfo = { path: DEFAULT_ROOT, source: "detected", exists: true, writable: true };
  const rootListeners = new Set<(e: { path: string | null }) => void>();
  const changeListeners = new Set<(e: { reason: string; platform: string; group: string; course?: string }) => void>();
  const pick = params.get("pick");
  const del = params.get("delete");

  const find = (group: string) => allGroups(library).find((g) => g.id === group);
  const courseOf = (groupId: string, folder?: string) => {
    const g = find(groupId);
    if (!g || !folder) return undefined;
    const n = Number(folder.split(" - ")[0]);
    return g.courses.find((c) => c.index === n);
  };

  return {
    libraryRoot: async () => ({ ...root }),
    librarySetRoot: async (path) => {
      if (path === PICKS.error) throw "the folder is not writable";
      root = { path, source: "saved", exists: true, writable: true };
      rootListeners.forEach((cb) => cb({ path }));
      return { ...root };
    },
    libraryResetRoot: async () => {
      root = { path: DEFAULT_ROOT, source: "detected", exists: true, writable: true };
      rootListeners.forEach((cb) => cb({ path: DEFAULT_ROOT }));
      return { ...root };
    },
    libraryPickRoot: async () => (pick && pick !== "cancel" ? PICKS[pick] ?? null : null),
    librarySummary: async (path) => SUMMARIES[path] ?? { platforms: [], hasLibrary: false },
    onLibraryRootChanged: async (cb) => {
      rootListeners.add(cb);
      return () => {
        rootListeners.delete(cb);
      };
    },
    libraryDeletePreview: async (target): Promise<DeletePreview> => {
      const g = find(target.group);
      if (target.lesson) {
        const l = g?.courses.flatMap((x) => x.sections.flatMap((sec) => sec.lessons)).find((x) => x.key === target.lesson);
        if (!g || !l || del === "notfound") throw { code: "lesson_not_found", message: "lesson not found" };
        return {
          name: l.title,
          platform: g.platform,
          kind: "lesson",
          lessons: 1,
          lessonsWithVideo: l.hasVideo ? 1 : 0,
          bytes: 87_400_000,
          bytesApprox: false,
          path: l.video ?? "",
          runningDownload: del === "running",
        };
      }
      const c = courseOf(target.group, target.course);
      if (!g || (target.course && !c) || del === "notfound") throw { code: "not_found", message: "not found" };
      const lessons = (c ? [c] : g.courses).flatMap((x) => x.sections.flatMap((s) => s.lessons));
      const withVideo = lessons.filter((l) => l.hasVideo).length;
      return {
        name: c ? c.title : g.title,
        platform: g.platform,
        kind: c ? "course" : "group",
        lessons: lessons.length,
        lessonsWithVideo: withVideo,
        bytes: withVideo * 87_400_000 + lessons.length * 120_000,
        bytesApprox: lessons.length > 200,
        path: `${g.platform === "skool" ? "Skool" : g.platform === "hotmart" ? "Hotmart" : "Pluralsight"}/${g.name}${target.course ? `/${target.course}` : ""}`,
        runningDownload: del === "running",
      };
    },
    libraryDelete: async (target, permanent) => {
      if (del === "running") throw { code: "download_running", message: "a download is running" };
      if (del === "trashfail" && !permanent) throw { code: "trash_failed", message: "trash is not available" };
      const g = find(target.group);
      if (!g) throw { code: "not_found", message: "not found" };
      if (target.lesson) {
        const key = target.lesson;
        let found = false;
        for (const course of g.courses)
          for (const sec of course.sections) {
            const before = sec.lessons.length;
            sec.lessons = sec.lessons.filter((x) => x.key !== key);
            found ||= sec.lessons.length < before;
          }
        if (!found) throw { code: "lesson_not_found", message: "lesson not found" };
        g.lessonCount = g.courses.reduce((n, x) => n + x.sections.reduce((m, sec) => m + sec.lessons.length, 0), 0);
        const groupRemoved = g.lessonCount === 0;
        if (groupRemoved) for (const p of library.platforms) p.groups = p.groups.filter((x) => x.id !== g.id);
        library.groups = library.platforms.find((p) => p.id === "skool")?.groups ?? [];
        changeListeners.forEach((cb) => cb({ reason: "deleted", platform: g.platform, group: g.id }));
        return { deleted: true, movedToTrash: !permanent, bytesFreed: 87_400_000, bytesApprox: false, groupRemoved };
      }
      const c = courseOf(target.group, target.course);
      const lessons = (c ? [c] : g.courses).flatMap((x) => x.sections.flatMap((s) => s.lessons));
      const bytes = lessons.filter((l) => l.hasVideo).length * 87_400_000;
      for (const p of library.platforms) {
        if (c) {
          const grp = p.groups.find((x) => x.id === g.id);
          if (grp) grp.courses = grp.courses.filter((x) => x !== c);
        } else p.groups = p.groups.filter((x) => x.id !== g.id);
      }
      library.groups = library.platforms.find((p) => p.id === "skool")?.groups ?? [];
      changeListeners.forEach((cb) => cb({ reason: "deleted", platform: g.platform, group: g.id, course: target.course }));
      return { deleted: true, movedToTrash: !permanent, bytesFreed: bytes, bytesApprox: false, groupRemoved: !c };
    },
    onLibraryChanged: async (cb) => {
      changeListeners.add(cb);
      return () => {
        changeListeners.delete(cb);
      };
    },
  };
}
