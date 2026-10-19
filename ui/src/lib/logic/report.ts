// Turns a download report into what the result panel shows.
import type { Issue, IssueKind, ReportEvent } from "../types";

export type ReportState = "complete" | "warnings" | "errors" | "cancelled";

export interface IssueGroup {
  kind: IssueKind;
  issues: Issue[];
}

export interface ReportSummary {
  state: ReportState;
  ok: number;
  total: number;
  errorCount: number;
  warningCount: number;
  errors: IssueGroup[];
  warnings: IssueGroup[];
}

const KIND_ORDER: readonly IssueKind[] = ["course", "module_page", "video", "video_not_resolved", "resource", "empty_lesson"];

function group(issues: Issue[]): IssueGroup[] {
  return KIND_ORDER.map((kind) => ({
    kind,
    issues: issues
      .filter((i) => i.kind === kind)
      .sort((a, b) => (a.lesson ?? "").localeCompare(b.lesson ?? "", "es", { numeric: true })),
  })).filter((g) => g.issues.length);
}

export function summarizeReport(r: ReportEvent): ReportSummary {
  const errors = r.issues.filter((i) => i.severity === "error");
  const warnings = r.issues.filter((i) => i.severity === "warning");
  const state: ReportState = r.cancelled
    ? "cancelled"
    : errors.length || !r.complete
      ? "errors"
      : warnings.length
        ? "warnings"
        : "complete";
  return {
    state,
    ok: Math.min(r.completedLessons, r.totalLessons),
    total: r.totalLessons,
    errorCount: errors.length,
    warningCount: warnings.length,
    errors: group(errors),
    warnings: group(warnings),
  };
}
