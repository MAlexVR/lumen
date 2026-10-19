// Udemy part of the preview mock. Presets `?udemy=connected|waiting|bot|result|cancelled|running`.
// The diagnostic replays a progress sequence and ends with a realistic result.
import type { Api } from "../api";
import type { DownloadError, UdemyCourseResult, UdemyProgress, UdemyReport, UdemySummary } from "../types";

type UdemyApi = Pick<
  Api,
  | "udemyLoginCookies"
  | "udemyLoginWebview"
  | "udemySessionStatus"
  | "udemyLogout"
  | "udemyDiagnose"
  | "udemyDiagnoseCancel"
  | "udemyDiagnosticLast"
  | "onUdemyProgress"
>;

const err = (code: DownloadError["code"], message: string): DownloadError => ({ code, message });
const cls = (d: number, drm: number, a = 0, q = 0) => ({ downloadable: d, drm, article: a, external: 0, quiz: q, unknown: 0 });
const course = (id: number, verdict: UdemyCourseResult["verdict"], video: number, article: number, quiz: number, c: UdemyCourseResult["classification"]): UdemyCourseResult => ({
  id,
  title: null,
  lectures: video + article + quiz,
  types: { video, article, quiz, other: 0 },
  probed: 5,
  classification: c,
  verdict,
});

const COURSES: UdemyCourseResult[] = [
  course(3104822, "downloadable", 42, 3, 2, cls(5, 0)),
  course(2219004, "downloadable", 88, 0, 0, cls(5, 0)),
  course(1877632, "downloadable", 31, 6, 1, cls(4, 0, 1)),
  course(4410215, "partial", 56, 2, 4, cls(3, 2)),
  course(3990127, "partial", 24, 0, 0, cls(2, 3)),
  course(2765513, "none", 63, 0, 0, cls(0, 5)),
  course(4012230, "none", 19, 1, 0, cls(0, 4, 1)),
  course(1550348, "unknown", 12, 0, 0, { downloadable: 0, drm: 0, article: 0, external: 0, quiz: 0, unknown: 3 }),
];

const ENDPOINTS = [
  { name: "subscribedCourses", status: "ok", verified: true, httpStatus: 200, note: "" },
  { name: "curriculumItems", status: "ok", verified: true, httpStatus: 200, note: "" },
  { name: "lectureDetail", status: "http_403", verified: false, httpStatus: 403, note: "1 course answered 403" },
];

function summary(status: UdemySummary["status"], courses = COURSES): UdemySummary {
  const v = { downloadable: 0, partial: 0, none: 0, unknown: 0 };
  for (const c of courses) v[c.verdict] += 1;
  return {
    status,
    courses: courses.length,
    lectures: courses.reduce((n, c) => n + (c.lectures ?? 0), 0),
    probed: courses.reduce((n, c) => n + c.probed, 0),
    classification: courses.reduce(
      (t, c) => {
        for (const k of Object.keys(t) as (keyof typeof t)[]) t[k] += c.classification[k];
        return t;
      },
      cls(0, 0),
    ),
    verdicts: v,
    endpoints: ENDPOINTS,
    issues: 9,
  };
}

function report(status: UdemySummary["status"], courses = COURSES): UdemyReport {
  return {
    generatedAt: "2026-10-17T10:42:00Z",
    status,
    options: { sampleLecturesPerCourse: 5, includeTitles: false, maxCourses: null },
    endpoints: ENDPOINTS,
    courses,
    totals: summary(status, courses),
    issues: [
      { lesson: null, kind: "video_not_resolved", severity: "warning", host: "www.udemy.com", url: "https://www.udemy.com/api-2.0/courses", message: "drm_protected: media_license_token present [2765513/5 lectures]" },
      { lesson: null, kind: "video_not_resolved", severity: "warning", host: "www.udemy.com", url: "https://www.udemy.com/api-2.0/courses", message: "drm_protected: course_is_drmed [4012230]" },
      { lesson: null, kind: "module_page", severity: "error", host: "www.udemy.com", url: "https://www.udemy.com/api-2.0/users/me/subscribed-courses", message: "lectureDetail: HTTP 403 for course 1550348" },
    ],
  };
}

const PATH = "/home/usuario/.local/share/com.malexvr.lumen/udemy-diagnostico.json";

export function createUdemyMock(preset: string | null): UdemyApi {
  let loggedIn = preset !== null && !["waiting", "bot"].includes(preset);
  let last: UdemyReport | null = preset === "result" ? report("complete") : preset === "cancelled" ? report("cancelled", COURSES.slice(0, 3)) : null;
  const listeners = new Set<(e: UdemyProgress) => void>();
  let cancelled = false;
  const emit = (e: UdemyProgress) => listeners.forEach((cb) => cb(e));
  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

  return {
    udemySessionStatus: async () => ({ loggedIn }),
    udemyLoginWebview: () =>
      preset === "waiting"
        ? new Promise(() => {})
        : preset === "bot"
          ? Promise.reject(err("bot_challenge", "udemy answered with a bot challenge"))
          : sleep(1500).then(() => {
              loggedIn = true;
              return { loggedIn, totalCourses: COURSES.length };
            }),
    udemyLoginCookies: async (text) => {
      if (!/access_token=/.test(text)) throw err("invalid_cookies", "access_token missing");
      loggedIn = true;
      return { loggedIn, totalCourses: COURSES.length };
    },
    udemyLogout: async () => {
      loggedIn = false;
    },
    udemyDiagnose: async (options) => {
      if (!loggedIn) throw err("not_logged_in", "not logged in");
      cancelled = false;
      const steps: UdemyProgress[] = [
        { step: "session", current: 1, total: 1, message: "Sesión comprobada" },
        { step: "courses", current: 8, total: 8, message: "8 cursos encontrados" },
        ...COURSES.map((_, i) => ({ step: "curriculum" as const, current: i + 1, total: 8, message: `Organización del curso ${i + 1} de 8` })),
        ...Array.from({ length: 40 }, (_, i) => ({ step: "lectures" as const, current: i + 1, total: 40, message: `Clase de muestra ${i + 1} de 40` })),
      ];
      const stopAt = preset === "running" ? 30 : Number.POSITIVE_INFINITY;
      for (const [i, s] of steps.entries()) {
        if (cancelled) break;
        if (i >= stopAt) await new Promise(() => {});
        emit(s);
        await sleep(60);
      }
      emit({ step: "finish", current: 0, total: 0, message: cancelled ? "Cancelado" : "Listo" });
      const status = cancelled ? "cancelled" : "complete";
      const courses = cancelled ? COURSES.slice(0, 3) : COURSES.slice(0, options.maxCourses ?? COURSES.length);
      last = report(status, courses);
      return { path: PATH, summary: summary(status, courses) };
    },
    udemyDiagnoseCancel: async () => {
      cancelled = true;
      return true;
    },
    udemyDiagnosticLast: async () => (last ? structuredClone(last) : null),
    onUdemyProgress: async (cb) => {
      listeners.add(cb);
      return () => {
        listeners.delete(cb);
      };
    },
  };
}

export const UDEMY_REPORT_PATH = PATH;
