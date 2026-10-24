// Turns a download report into what the result panel shows.
import type { MessageKey } from "../i18n/index.svelte";
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

const NO_MODULES = /course has no modules/i;

/** i18n keys that replace a backend message we know how to explain. */
export interface IssueNote {
  message: MessageKey;
  hint: MessageKey;
}

/** A course that came back without modules (Hotmart, Pluralsight): say what to do. */
export function issueNote(issue: Issue): IssueNote | null {
  if (issue.kind === "course" && NO_MODULES.test(issue.message)) {
    return { message: "dl_issue_no_modules", hint: "dl_issue_no_modules_hint" };
  }
  return null;
}
