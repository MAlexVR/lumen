// Udemy connection wizard: steps, diagnostic progress, and how the result is
// summarised. Nothing here downloads; the diagnostic only reads metadata.
import type { UdemyCourseResult, UdemyProgress, UdemySummary, UdemyVerdict } from "../types";

export type WizardStep = "login" | "options" | "running" | "result";
export type WizardState = { step: WizardStep };
export type WizardAction =
  | { type: "resume"; loggedIn: boolean; hasReport: boolean }
  | { type: "loggedIn" }
  | { type: "start" }
  | { type: "finished" }
  | { type: "back" }
  | { type: "sessionLost" };

export const WIZARD_STEPS: readonly WizardStep[] = ["login", "options", "running", "result"];
export const initialWizard: WizardState = { step: "login" };

export function reduceWizard(state: WizardState, action: WizardAction): WizardState {
  switch (action.type) {
    case "resume":
      return { step: !action.loggedIn ? "login" : action.hasReport ? "result" : "options" };
    case "loggedIn":
      return { step: "options" };
    case "start":
      return state.step === "options" || state.step === "result" ? { step: "running" } : state;
    case "finished":
      return { step: "result" };
    case "back":
      if (state.step === "options") return { step: "login" };
      if (state.step === "result") return { step: "options" };
      return state;
    case "sessionLost":
      return { step: "login" };
  }
}

type StepName = Exclude<UdemyProgress["step"], "finish">;
export const PROGRESS_STEPS: readonly StepName[] = ["session", "courses", "curriculum", "lectures"];

export interface ProgressState {
  current: UdemyProgress["step"] | null;
  message: string;
  steps: Record<StepName, { current: number; total: number }>;
  done: boolean;
}

export const initialProgress: ProgressState = {
  current: null,
  message: "",
  steps: Object.fromEntries(PROGRESS_STEPS.map((s) => [s, { current: 0, total: 0 }])) as ProgressState["steps"],
  done: false,
};

export function reduceProgress(state: ProgressState, e: UdemyProgress): ProgressState {
  if (e.step === "finish") return { ...state, current: "finish", message: e.message || state.message, done: true };
  const prev = state.steps[e.step];
  const total = Math.max(0, e.total);
  const current = Math.min(total || e.current, Math.max(prev.current, e.current));
  return { ...state, current: e.step, message: e.message, steps: { ...state.steps, [e.step]: { current, total } } };
}

const VERDICT_ORDER: readonly UdemyVerdict[] = ["downloadable", "partial", "none", "unknown"];

/** "12 cursos: 7 descargables · 2 parciales · 3 con DRM" as i18n parts. */
export function summaryParts(s: Pick<UdemySummary, "courses" | "verdicts">): {
  courses: number;
  parts: { key: `ud_v_${UdemyVerdict}_${"one" | "n"}`; n: number }[];
} {
  return {
    courses: s.courses,
    parts: VERDICT_ORDER.filter((v) => s.verdicts[v] > 0).map((v) => ({
      key: `ud_v_${v}_${s.verdicts[v] === 1 ? "one" : "n"}` as const,
      n: s.verdicts[v],
    })),
  };
}

export function sortCourses(courses: readonly UdemyCourseResult[]): UdemyCourseResult[] {
  return [...courses].sort(
    (a, b) =>
      VERDICT_ORDER.indexOf(a.verdict) - VERDICT_ORDER.indexOf(b.verdict) ||
      (a.title ?? "").localeCompare(b.title ?? "", "es") ||
      a.id - b.id,
  );
}
