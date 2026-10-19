import { describe, expect, it } from "vitest";
import type { UdemyCourseResult, UdemySummary } from "../types";
import {
  initialProgress,
  initialWizard,
  reduceProgress,
  reduceWizard,
  sortCourses,
  summaryParts,
} from "./udemy";

describe("reduceWizard", () => {
  it("walks login → options → running → result", () => {
    let s = reduceWizard(initialWizard, { type: "loggedIn" });
    expect(s.step).toBe("options");
    s = reduceWizard(s, { type: "start" });
    expect(s.step).toBe("running");
    s = reduceWizard(s, { type: "finished" });
    expect(s.step).toBe("result");
  });

  it("resumes at the result when a report exists and the user is logged in", () => {
    expect(reduceWizard(initialWizard, { type: "resume", loggedIn: true, hasReport: true }).step).toBe("result");
    expect(reduceWizard(initialWizard, { type: "resume", loggedIn: true, hasReport: false }).step).toBe("options");
    expect(reduceWizard(initialWizard, { type: "resume", loggedIn: false, hasReport: true }).step).toBe("login");
  });

  it("goes back one step, never from running, and to login on session loss", () => {
    expect(reduceWizard({ step: "options" }, { type: "back" }).step).toBe("login");
    expect(reduceWizard({ step: "result" }, { type: "back" }).step).toBe("options");
    expect(reduceWizard({ step: "running" }, { type: "back" }).step).toBe("running");
    expect(reduceWizard({ step: "running" }, { type: "sessionLost" }).step).toBe("login");
  });

  it("re-checks from the result", () => {
    expect(reduceWizard({ step: "result" }, { type: "start" }).step).toBe("running");
  });
});

describe("reduceProgress", () => {
  it("tracks each step's latest counts and message, and the current step", () => {
    let p = reduceProgress(initialProgress, { step: "session", current: 1, total: 1, message: "ok" });
    p = reduceProgress(p, { step: "courses", current: 3, total: 12, message: "Leyendo cursos" });
    expect(p.current).toBe("courses");
    expect(p.steps.courses).toEqual({ current: 3, total: 12 });
    expect(p.steps.session).toEqual({ current: 1, total: 1 });
    expect(p.message).toBe("Leyendo cursos");
  });

  it("never moves a step's counter backwards and clamps current to total", () => {
    let p = reduceProgress(initialProgress, { step: "lectures", current: 7, total: 10, message: "" });
    p = reduceProgress(p, { step: "lectures", current: 5, total: 10, message: "" });
    expect(p.steps.lectures.current).toBe(7);
    p = reduceProgress(p, { step: "lectures", current: 14, total: 10, message: "" });
    expect(p.steps.lectures.current).toBe(10);
  });

  it("marks finish as done", () => {
    expect(reduceProgress(initialProgress, { step: "finish", current: 0, total: 0, message: "" }).done).toBe(true);
  });
});

const summary = (v: Partial<UdemySummary["verdicts"]>, courses = 12): UdemySummary => ({
  status: "complete",
  courses,
  lectures: 0,
  probed: 0,
  classification: { downloadable: 0, drm: 0, article: 0, external: 0, quiz: 0, unknown: 0 },
  verdicts: { downloadable: 0, partial: 0, none: 0, unknown: 0, ...v },
  endpoints: [],
  issues: 0,
});

describe("summaryParts", () => {
  it("lists the non-zero verdicts in a stable order with plural keys", () => {
    expect(summaryParts(summary({ downloadable: 7, partial: 2, none: 3 }))).toEqual({
      courses: 12,
      parts: [
        { key: "ud_v_downloadable_n", n: 7 },
        { key: "ud_v_partial_n", n: 2 },
        { key: "ud_v_none_n", n: 3 },
      ],
    });
  });

  it("uses singular keys for one", () => {
    expect(summaryParts(summary({ partial: 1, unknown: 1 }, 2)).parts).toEqual([
      { key: "ud_v_partial_one", n: 1 },
      { key: "ud_v_unknown_one", n: 1 },
    ]);
  });

  it("has no parts for an empty account", () => {
    expect(summaryParts(summary({}, 0))).toEqual({ courses: 0, parts: [] });
  });
});

describe("sortCourses", () => {
  const c = (id: number, verdict: UdemyCourseResult["verdict"], title: string | null = null): UdemyCourseResult => ({
    id,
    title,
    lectures: 1,
    types: { video: 1, article: 0, quiz: 0, other: 0 },
    probed: 1,
    classification: { downloadable: 0, drm: 0, article: 0, external: 0, quiz: 0, unknown: 0 },
    verdict,
  });

  it("orders downloadable, partial, none, unknown, then by title or id", () => {
    const out = sortCourses([c(5, "none"), c(2, "downloadable", "B"), c(9, "unknown"), c(1, "downloadable", "A"), c(3, "partial")]);
    expect(out.map((x) => x.id)).toEqual([1, 2, 3, 5, 9]);
  });
});
