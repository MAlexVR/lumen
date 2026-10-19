// Deleting downloaded groups/courses: targets (never paths), the dialog flow,
// backend error codes and sizes.
import type { Course, DeleteError, DeleteErrorCode, DeletePreview, DeleteResult, DeleteTarget, Group } from "../types";

export type { DeletePreview };

/** Folder name of a Skool course on disk, read from its own lesson paths
 * (`Skool/<group>/<NN - course>/…`); falls back to `NN - title`. */
export function courseFolder(group: Group, course: Course): string {
  const prefix = `Skool/${group.name}/`;
  for (const s of course.sections)
    for (const l of s.lessons)
      for (const p of [l.video, l.description, ...l.resources.map((r) => r.path)]) {
        if (p?.startsWith(prefix)) {
          const folder = p.slice(prefix.length).split("/")[0];
          if (folder) return folder;
        }
      }
  return `${String(course.index).padStart(2, "0")} - ${course.title}`;
}

/** Hotmart purchases are deleted whole; Skool can drop one course. */
export function deleteTarget(group: Group, course?: Course): DeleteTarget {
  if (group.platform === "skool" && course) return { platform: "skool", group: group.id, course: courseFolder(group, course) };
  return { platform: group.platform, group: group.id };
}

const CODES: readonly DeleteErrorCode[] = ["lesson_not_found", "invalid_target", "not_found", "download_running", "trash_failed", "internal"];

export function toDeleteError(err: unknown): DeleteError {
  if (typeof err === "string") return { code: "internal", message: err };
  if (err && typeof err === "object") {
    const o = err as { code?: unknown; message?: unknown };
    const code = CODES.includes(o.code as DeleteErrorCode) ? (o.code as DeleteErrorCode) : "internal";
    return { code, message: typeof o.message === "string" ? o.message : "" };
  }
  return { code: "internal", message: "" };
}

const UNITS = ["B", "KB", "MB", "GB", "TB"];

/** Decimal units (as file managers show them); `~` when the size is approximate. */
export function formatBytes(bytes: number, approx: boolean, locale: string): string {
  let value = Math.max(0, bytes);
  let unit = 0;
  while (value >= 1000 && unit < UNITS.length - 1) {
    value /= 1000;
    unit += 1;
  }
  const n = new Intl.NumberFormat(locale, { maximumFractionDigits: unit === 0 ? 0 : 1 }).format(value);
  return `${approx ? "~" : ""}${n} ${UNITS[unit]}`;
}

export type DeleteState =
  | { step: "idle" }
  | { step: "previewing"; target: DeleteTarget; title: string }
  | { step: "confirming"; target: DeleteTarget; title: string; preview: DeletePreview }
  | { step: "deleting"; target: DeleteTarget; title: string; preview: DeletePreview; permanent: boolean }
  | { step: "done"; target: DeleteTarget; title: string; preview: DeletePreview; result: DeleteResult }
  | { step: "error"; target: DeleteTarget; title: string; preview: DeletePreview | null; error: DeleteError };

export type DeleteAction =
  | { type: "open"; target: DeleteTarget; title: string }
  | { type: "previewed"; preview: DeletePreview }
  | { type: "confirm"; permanent: boolean }
  | { type: "deleted"; result: DeleteResult }
  | { type: "failed"; error: DeleteError }
  | { type: "close" };

export const initialDelete: DeleteState = { step: "idle" };

export function reduceDelete(state: DeleteState, action: DeleteAction): DeleteState {
  switch (action.type) {
    case "open":
      return { step: "previewing", target: action.target, title: action.title };
    case "previewed":
      return state.step === "previewing" ? { ...state, step: "confirming", preview: action.preview } : state;
    case "confirm": {
      if (state.step !== "confirming" && !(state.step === "error" && state.preview)) return state;
      const preview = state.preview;
      if (!preview || preview.runningDownload) return state;
      return { step: "deleting", target: state.target, title: state.title, preview, permanent: action.permanent };
    }
    case "deleted":
      return state.step === "deleting" ? { ...state, step: "done", result: action.result } : state;
    case "failed":
      if (state.step === "idle") return state;
      return {
        step: "error",
        target: state.target,
        title: state.title,
        preview: "preview" in state ? state.preview : null,
        error: action.error,
      };
    case "close":
      return initialDelete;
  }
}
