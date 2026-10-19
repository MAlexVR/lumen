import { describe, expect, it } from "vitest";
import type { Issue, ReportEvent } from "../types";
import { summarizeReport } from "./report";

const issue = (i: Partial<Issue>): Issue => ({
  lesson: "L",
  kind: "video",
  severity: "error",
  host: null,
  url: null,
  message: "m",
  ...i,
});

const report = (r: Partial<ReportEvent>): ReportEvent => ({
  id: "dl-1",
  platform: "skool",
  group: "G",
  cancelled: false,
  totalLessons: 10,
  completedLessons: 10,
  complete: true,
  issues: [],
  ...r,
});

describe("summarizeReport", () => {
  it("is complete without issues", () => {
    expect(summarizeReport(report({}))).toMatchObject({ state: "complete", ok: 10, total: 10, errorCount: 0, warningCount: 0 });
  });

  it("is complete with warnings when only warnings exist", () => {
    const s = summarizeReport(report({ issues: [issue({ severity: "warning", kind: "empty_lesson" })] }));
    expect(s).toMatchObject({ state: "warnings", warningCount: 1, errorCount: 0 });
  });

  it("reports errors when any error exists or the group is not complete", () => {
    expect(summarizeReport(report({ complete: false, completedLessons: 8, issues: [issue({})] })).state).toBe("errors");
    expect(summarizeReport(report({ complete: false })).state).toBe("errors");
  });

  it("reports cancellation first", () => {
    expect(summarizeReport(report({ cancelled: true, issues: [issue({})] })).state).toBe("cancelled");
  });

  it("groups by severity then kind in a stable order and sorts by lesson", () => {
    const s = summarizeReport(
      report({
        complete: false,
        issues: [
          issue({ kind: "resource", lesson: "B" }),
          issue({ kind: "video", lesson: "Z" }),
          issue({ kind: "video", lesson: "A" }),
          issue({ kind: "empty_lesson", severity: "warning", lesson: "C" }),
          issue({ kind: "course", lesson: null }),
        ],
      }),
    );
    expect(s.errors.map((g) => g.kind)).toEqual(["course", "video", "resource"]);
    expect(s.errors[1].issues.map((i) => i.lesson)).toEqual(["A", "Z"]);
    expect(s.warnings.map((g) => g.kind)).toEqual(["empty_lesson"]);
  });

  it("clamps impossible counts", () => {
    expect(summarizeReport(report({ completedLessons: 12, totalLessons: 10 })).ok).toBe(10);
  });
});
