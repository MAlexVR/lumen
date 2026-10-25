// Skool and Hotmart parts of the preview mock, sharing one event bus so
// download ids are unique across platforms (as in the backend).
// `?skool=<preset>` / `?hotmart=<preset>` select a starting state for
// screenshots: connected, waiting, downloading, report-ok, report-warn,
// report-err, cancelled, nolibrary. Default: not connected.
import type { Api } from "../api";
import type {
  CourseInfo,
  DownloadError,
  Issue,
  PlatformGroup,
  PlatformId,
  PluralsightCourse,
  GoldenlifeCourse,
  ProgressEvent,
  ReportEvent,
  RunningDownload,
} from "../types";

type Bus = ReturnType<typeof createBus>;
type Outcome = "ok" | "warn" | "err" | "cancelled" | "ytwarn";

const err = (code: DownloadError["code"], message: string): DownloadError => ({ code, message });

const LESSONS = [
  "Bienvenida",
  "Cómo funciona el curso",
  "Organiza tu semana",
  "Materiales y herramientas",
  "Primeros ejercicios",
  "Estrategia de práctica",
  "Resumen del módulo",
  "Ejemplos de referencia",
  "Errores frecuentes",
  "Revisión con casos reales",
  "Plantillas descargables",
  "Cierre y próximos pasos",
];

const ISSUES: Record<Exclude<Outcome, "cancelled">, Issue[]> = {
  ok: [],
  warn: [
    { lesson: "Resumen del módulo", kind: "empty_lesson", severity: "warning", host: null, url: null, message: "lesson has no video, text or files" },
    { lesson: "Bienvenida", kind: "empty_lesson", severity: "warning", host: null, url: null, message: "lesson has no video, text or files" },
    { lesson: null, kind: "course", severity: "warning", host: null, url: null, message: "course is locked for this account" },
  ],
  ytwarn: [
    { lesson: null, kind: "video_not_resolved", severity: "warning", host: "www.youtube.com", url: "https://www.youtube.com/watch", message: "private video: Private video. Sign in if you've been granted access to this video [Xp4Mq9vR2tA]" },
  ],
  err: [
    { lesson: "Plantillas descargables", kind: "video_not_resolved", severity: "error", host: "vimeo.com", url: "https://vimeo.com/812345678", message: "video page did not expose a playable stream" },
    { lesson: "Cierre y próximos pasos", kind: "video", severity: "error", host: "player.mux.com", url: "https://player.mux.com/a1b2c3", message: "yt-dlp exited with status 1" },
    { lesson: "Ejemplos de referencia", kind: "resource", severity: "error", host: "drive.google.com", url: "https://drive.google.com/file/d/1AbC", message: "HTTP 403 Forbidden" },
    { lesson: "Resumen del módulo", kind: "empty_lesson", severity: "warning", host: null, url: null, message: "lesson has no video, text or files" },
  ],
};

const REPORT_PRESETS: Record<string, Outcome> = { "report-ok": "ok", "report-warn": "warn", "report-err": "err", cancelled: "cancelled" };

export function createBus() {
  const progress = new Set<(e: ProgressEvent) => void>();
  const reports = new Set<(e: ReportEvent) => void>();
  const running = new Map<string, { platform: PlatformId; key: string; name: string; timer?: ReturnType<typeof setInterval> }>();
  let counter = 1;

  function finish(id: string, outcome: Outcome, done: number, total = LESSONS.length) {
    const r = running.get(id);
    clearInterval(r?.timer);
    running.delete(id);
    const e: ReportEvent = {
      id,
      platform: r?.platform ?? "skool",
      group: r?.name ?? "",
      cancelled: outcome === "cancelled",
      totalLessons: total,
      completedLessons: done,
      complete: outcome === "ok" || outcome === "warn" || outcome === "ytwarn",
      issues: outcome === "cancelled" ? [] : ISSUES[outcome],
    };
    reports.forEach((cb) => cb(e));
  }

  /** One lesson every ~1.2 s in four progress steps. */
  function simulate(
    platform: PlatformId,
    key: string,
    name: string,
    opts: { startAt?: number; outcome?: Exclude<Outcome, "cancelled">; modules?: [string, string]; total?: number } = {},
  ): string {
    const id = `dl-${counter++}`;
    const outcome = opts.outcome ?? "warn";
    const [m1, m2] = opts.modules ?? ["Hábitos de los que avanzan", "Conoce las herramientas básicas"];
    let step = (opts.startAt ?? 0) * 4;
    const entry: { platform: PlatformId; key: string; name: string; timer?: ReturnType<typeof setInterval> } = { platform, key, name };
    running.set(id, entry);
    entry.timer = setInterval(() => {
      const completed = Math.floor(step / 4);
      const total = opts.total ?? LESSONS.length;
      if (completed >= total) {
        finish(id, outcome, outcome === "err" ? total - 3 : total, total);
        return;
      }
      const e: ProgressEvent = {
        id,
        platform,
        group: name,
        course: completed < 6 ? m1 : m2,
        lesson: LESSONS[completed],
        percent: (step % 4) * 25,
        completed,
        total,
      };
      progress.forEach((cb) => cb(e));
      step += 1;
    }, 300);
    return id;
  }

  /** A download that reports `outcome` right away (report presets). */
  function finished(platform: PlatformId, key: string, name: string, outcome: Outcome, total = LESSONS.length) {
    const id = `dl-${counter++}`;
    running.set(id, { platform, key, name });
    const done = outcome === "err" ? total - 3 : outcome === "cancelled" ? 5 : outcome === "ytwarn" ? total - 1 : total;
    setTimeout(() => finish(id, outcome, done, total), 400);
    return id;
  }

  return {
    simulate,
    finished,
    isRunning: (platform: PlatformId, key: string) =>
      [...running.values()].some((r) => r.platform === platform && r.key === key),
    runningOf: (platform: PlatformId): RunningDownload[] =>
      [...running.entries()].filter(([, r]) => r.platform === platform).map(([id, r]) => ({ id, platform, group: r.key })),
    cancel(id: string, platform: PlatformId): boolean {
      const r = running.get(id);
      if (!r || r.platform !== platform) return false;
      clearInterval(r.timer);
      setTimeout(() => finish(id, "cancelled", 3), 300);
      return true;
    },
    onProgress: async (cb: (e: ProgressEvent) => void) => {
      progress.add(cb);
      return () => {
        progress.delete(cb);
      };
    },
    onReport: async (cb: (e: ReportEvent) => void) => {
      reports.add(cb);
      return () => {
        reports.delete(cb);
      };
    },
  };
}

// Skool ---------------------------------------------------------------------

const GROUPS: PlatformGroup[] = [
  { slug: "academia-aurora", name: "ACADEMIA AURORA", coverUrl: null, numCourses: 11 },
  { slug: "estudio", name: "Estudio", coverUrl: null, numCourses: 3 },
  { slug: "growth-lab", name: "🚀 Growth Lab", coverUrl: null, numCourses: 6 },
  { slug: "copy-academy", name: "Copywriting Academy", coverUrl: null, numCourses: 4 },
];

type SkoolApi = Pick<
  Api,
  | "skoolLoginCookies"
  | "skoolLoginWebview"
  | "skoolSessionStatus"
  | "skoolLogout"
  | "skoolGroups"
  | "skoolDownloadStart"
  | "skoolDownloadCancel"
  | "skoolDownloadsRunning"
>;

export function createSkoolMock(preset: string | null, bus: Bus): SkoolApi {
  let loggedIn = preset !== null && preset !== "waiting";
  const nameOf = (slug: string) => GROUPS.find((g) => g.slug === slug)?.name ?? slug;
  if (preset === "downloading") {
    bus.simulate("skool", "academia-aurora", nameOf("academia-aurora"), { startAt: 4 });
    bus.simulate("skool", "growth-lab", nameOf("growth-lab"), { startAt: 9, outcome: "ok" });
  }
  if (preset && REPORT_PRESETS[preset]) bus.finished("skool", "academia-aurora", nameOf("academia-aurora"), REPORT_PRESETS[preset]);

  return {
    skoolSessionStatus: async () => ({ loggedIn }),
    skoolLoginWebview: () =>
      preset === "waiting"
        ? new Promise(() => {})
        : new Promise((resolve) =>
            setTimeout(() => {
              loggedIn = true;
              resolve({ loggedIn, groups: structuredClone(GROUPS) });
            }, 2500),
          ),
    skoolLoginCookies: async (text) => {
      if (!/auth_token=/.test(text)) throw err("invalid_cookies", "auth_token missing");
      loggedIn = true;
      return { loggedIn, groups: structuredClone(GROUPS) };
    },
    skoolLogout: async () => {
      loggedIn = false;
    },
    skoolGroups: async () => {
      if (!loggedIn) throw err("not_logged_in", "not logged in");
      return structuredClone(GROUPS);
    },
    skoolDownloadStart: async (slug) => {
      if (!loggedIn) throw err("not_logged_in", "not logged in");
      if (preset === "nolibrary") throw err("no_library", "choose a library folder first");
      if (bus.isRunning("skool", slug)) throw err("already_running", "already running");
      return bus.simulate("skool", slug, nameOf(slug));
    },
    skoolDownloadCancel: async (id) => bus.cancel(id, "skool"),
    skoolDownloadsRunning: async () => bus.runningOf("skool"),
  };
}

// Hotmart -------------------------------------------------------------------

/** Invented purchases; folders match the synthetic Hotmart groups in library.json. */
const COURSES: CourseInfo[] = [
  { id: "4710021", name: "Fotografía para Emprendedores", seller: "Estudio Norte", coverUrl: null, purchaseDate: "2024-03-18", folder: "Fotografía para Emprendedores - Estudio Norte" },
  { id: "4710022", name: "Marketing de Contenidos Avanzado", seller: "Academia Faro", coverUrl: null, purchaseDate: "2024-07-02", folder: "Marketing de Contenidos Avanzado - Academia Faro" },
  { id: "4710023", name: "Finanzas Personales desde Cero", seller: "Escuela Prisma", coverUrl: null, purchaseDate: "2025-01-11", folder: "Finanzas Personales desde Cero - Escuela Prisma" },
  { id: "4710024", name: "Oratoria y Presentaciones", seller: null, coverUrl: null, purchaseDate: null, folder: "Oratoria y Presentaciones" },
  { id: "4710025", name: "Diseño de Producto Digital", seller: "Taller Ámbar", coverUrl: null, purchaseDate: "2025-05-27", folder: "Diseño de Producto Digital - Taller Ámbar" },
];

type HotmartApi = Pick<
  Api,
  | "hotmartLoginCredentials"
  | "hotmartLoginWebview"
  | "hotmartSessionStatus"
  | "hotmartLogout"
  | "hotmartCourses"
  | "hotmartDownloadStart"
  | "hotmartDownloadCancel"
  | "hotmartDownloadsRunning"
>;

const MODULES: [string, string] = ["Fundamentos", "Estrategia"];

export function createHotmartMock(preset: string | null, bus: Bus): HotmartApi {
  let loggedIn = preset !== null && preset !== "waiting";
  const nameOf = (id: string) => COURSES.find((c) => c.id === id)?.name ?? id;
  if (preset === "downloading") {
    bus.simulate("hotmart", "4710022", nameOf("4710022"), { startAt: 3, modules: MODULES });
    bus.simulate("hotmart", "4710025", nameOf("4710025"), { startAt: 8, outcome: "ok", modules: MODULES });
  }
  if (preset && REPORT_PRESETS[preset]) bus.finished("hotmart", "4710022", nameOf("4710022"), REPORT_PRESETS[preset]);

  return {
    hotmartSessionStatus: async () => ({ loggedIn }),
    hotmartLoginWebview: () =>
      preset === "waiting"
        ? new Promise(() => {})
        : new Promise((resolve) =>
            setTimeout(() => {
              loggedIn = true;
              resolve({ loggedIn, courses: structuredClone(COURSES) });
            }, 2500),
          ),
    hotmartLoginCredentials: async (token) => {
      if (token.trim().length < 20) throw err("invalid_token", "token too short");
      loggedIn = true;
      return { loggedIn, courses: structuredClone(COURSES) };
    },
    hotmartLogout: async () => {
      loggedIn = false;
    },
    hotmartCourses: async () => {
      if (!loggedIn) throw err("not_logged_in", "not logged in");
      return structuredClone(COURSES);
    },
    hotmartDownloadStart: async (course) => {
      if (!loggedIn) throw err("not_logged_in", "not logged in");
      if (preset === "nolibrary") throw err("no_library", "choose a library folder first");
      if (bus.isRunning("hotmart", course)) throw err("already_running", "already running");
      return bus.simulate("hotmart", course, nameOf(course), { modules: MODULES });
    },
    hotmartDownloadCancel: async (id) => bus.cancel(id, "hotmart"),
    hotmartDownloadsRunning: async () => bus.runningOf("hotmart"),
  };
}


// Pluralsight ---------------------------------------------------------------

const PLURALSIGHT_COURSES: PluralsightCourse[] = [
  { id: "course-js-101", title: "JavaScript Fundamentals", level: "Beginner", imageUrl: null, folder: "JavaScript Fundamentals-0123456789abcdef", issue: null },
  { id: null, title: "Unresolved history row", level: null, imageUrl: null, folder: "Unresolved history row-0123456789abcdef", issue: "The history entry did not include a course identifier." },
];

type PluralsightApi = Pick<
  Api,
  | "pluralsightLoginCredentials"
  | "pluralsightLoginWebview"
  | "pluralsightSessionStatus"
  | "pluralsightLogout"
  | "pluralsightCourses"
  | "pluralsightDownloadStart"
  | "pluralsightDownloadCancel"
  | "pluralsightDownloadsRunning"
>;

export function createPluralsightMock(preset: string | null, bus: Bus): PluralsightApi {
  let loggedIn = preset !== null && preset !== "waiting";
  const courseName = (id: string) => PLURALSIGHT_COURSES.find((course) => course.id === id)?.title ?? id;
  if (preset === "downloading") bus.simulate("pluralsight", "course-js-101", courseName("course-js-101"));
  if (preset && REPORT_PRESETS[preset]) bus.finished("pluralsight", "course-js-101", courseName("course-js-101"), REPORT_PRESETS[preset]);

  return {
    pluralsightSessionStatus: async () => ({ loggedIn }),
    pluralsightLoginWebview: () =>
      preset === "waiting"
        ? new Promise(() => {})
        : new Promise((resolve) => setTimeout(() => {
            loggedIn = true;
            resolve({ loggedIn, courses: structuredClone(PLURALSIGHT_COURSES) });
          }, 2500)),
    pluralsightLoginCredentials: async (text) => {
      if (!/(?:^|;\s*)session=/.test(text)) throw err("invalid_cookies", "session cookie missing");
      loggedIn = true;
      return { loggedIn, courses: structuredClone(PLURALSIGHT_COURSES) };
    },
    pluralsightLogout: async () => {
      loggedIn = false;
    },
    pluralsightCourses: async () => {
      if (!loggedIn) throw err("not_logged_in", "not logged in");
      return structuredClone(PLURALSIGHT_COURSES);
    },
    pluralsightDownloadStart: async (course) => {
      if (!loggedIn) throw err("not_logged_in", "not logged in");
      if (preset === "nolibrary") throw err("no_library", "choose a library folder first");
      if (!PLURALSIGHT_COURSES.some((item) => item.id === course)) throw err("invalid_cookies", "course cannot be downloaded");
      if (bus.isRunning("pluralsight", course)) throw err("already_running", "already running");
      return bus.simulate("pluralsight", course, courseName(course));
    },
    pluralsightDownloadCancel: async (id) => bus.cancel(id, "pluralsight"),
    pluralsightDownloadsRunning: async () => bus.runningOf("pluralsight"),
  };
}

// Golden Life ---------------------------------------------------------------

const GOLDENLIFE_COURSES: GoldenlifeCourse[] = [
  { id: "101", title: "Creator Basics", shelf: "UGC", imageUrl: null, folder: "Creator Basics", issue: null },
  { id: "102", title: "Phone video editing", shelf: "UGC", imageUrl: null, folder: "Phone video editing", issue: null },
  { id: "201", title: "Personal branding on social media", shelf: "CREADOR DIGITAL", imageUrl: null, folder: "Marca personal en redes", issue: null },
  { id: null, title: "Reto VIP", shelf: "CREADOR DIGITAL", imageUrl: null, folder: "Reto VIP", issue: "This course has no content of its own on the academy." },
];

type GoldenlifeApi = Pick<
  Api,
  | "goldenlifeLoginCredentials"
  | "goldenlifeSessionStatus"
  | "goldenlifeLogout"
  | "goldenlifeCourses"
  | "goldenlifeDownloadStart"
  | "goldenlifeDownloadCancel"
  | "goldenlifeDownloadsRunning"
>;

export function createGoldenlifeMock(preset: string | null, bus: Bus): GoldenlifeApi {
  let loggedIn = preset !== null;
  const courseName = (id: string) => GOLDENLIFE_COURSES.find((course) => course.id === id)?.title ?? id;
  if (preset === "downloading") bus.simulate("goldenlife", "101", courseName("101"));
  if (preset && REPORT_PRESETS[preset]) bus.finished("goldenlife", "101", courseName("101"), REPORT_PRESETS[preset]);

  return {
    goldenlifeSessionStatus: async () => ({ loggedIn }),
    // The preview accepts any password except the literal "wrong".
    goldenlifeLoginCredentials: async (_email, password) => {
      if (password === "wrong") throw err("invalid_credentials", "wrong email or password");
      loggedIn = true;
      return { loggedIn, courses: structuredClone(GOLDENLIFE_COURSES) };
    },
    goldenlifeLogout: async () => {
      loggedIn = false;
    },
    goldenlifeCourses: async () => {
      if (!loggedIn) throw err("not_logged_in", "not logged in");
      return structuredClone(GOLDENLIFE_COURSES);
    },
    goldenlifeDownloadStart: async (course) => {
      if (!loggedIn) throw err("not_logged_in", "not logged in");
      if (preset === "nolibrary") throw err("no_library", "choose a library folder first");
      if (!GOLDENLIFE_COURSES.some((item) => item.id === course)) throw err("internal", "course cannot be downloaded");
      if (bus.isRunning("goldenlife", course)) throw err("already_running", "already running");
      return bus.simulate("goldenlife", course, courseName(course));
    },
    goldenlifeDownloadCancel: async (id) => bus.cancel(id, "goldenlife"),
    goldenlifeDownloadsRunning: async () => bus.runningOf("goldenlife"),
  };
}
