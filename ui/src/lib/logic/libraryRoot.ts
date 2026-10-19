// Library folder setting: backend error strings, the pick → review → save
// flow, and what to say about the chosen folder.
import type { LibrarySummary } from "../types";

export type RootErrorKind =
  | "empty"
  | "not_absolute"
  | "forbidden_root"
  | "forbidden_system"
  | "forbidden_home"
  | "not_directory"
  | "not_writable"
  | "io"
  | "unknown";

/** library_set_root rejects with plain strings (src-tauri library_root.rs). */
export function rootErrorKind(err: unknown): RootErrorKind {
  const m = typeof err === "string" ? err : err instanceof Error ? err.message : "";
  if (m === "choose a folder") return "empty";
  if (m.includes("must be absolute")) return "not_absolute";
  if (m.includes("cannot be used as a library")) {
    if (m.includes("filesystem root")) return "forbidden_root";
    if (m.includes("system folder")) return "forbidden_system";
    if (m.includes("home folder")) return "forbidden_home";
    return "forbidden_system";
  }
  if (m.includes("is a file, not a folder")) return "not_directory";
  if (m.includes("not writable")) return "not_writable";
  if (m.startsWith("cannot use that folder")) return "io";
  return "unknown";
}

export function summaryMessage(s: LibrarySummary): { key: "lr_summary_has" | "lr_summary_empty"; platforms: string } {
  return s.hasLibrary && s.platforms.length
    ? { key: "lr_summary_has", platforms: s.platforms.join(", ") }
    : { key: "lr_summary_empty", platforms: "" };
}

export type PickerState =
  | { step: "idle" }
  | { step: "picking" }
  | { step: "reviewing"; path: string; summary: LibrarySummary | null }
  | { step: "saving"; path: string }
  | { step: "error"; path: string | null; kind: RootErrorKind };

export type PickerAction =
  | { type: "pick" }
  | { type: "picked"; path: string | null }
  | { type: "summary"; summary: LibrarySummary }
  | { type: "confirm" }
  | { type: "saved" }
  | { type: "failed"; kind: RootErrorKind }
  | { type: "cancel" };

export const initialPicker: PickerState = { step: "idle" };

export function reducePicker(state: PickerState, action: PickerAction): PickerState {
  switch (action.type) {
    case "pick":
      return { step: "picking" };
    case "picked":
      return action.path ? { step: "reviewing", path: action.path, summary: null } : initialPicker;
    case "summary":
      return state.step === "reviewing" ? { ...state, summary: action.summary } : state;
    case "confirm":
      return state.step === "reviewing" ? { step: "saving", path: state.path } : state;
    case "saved":
      return initialPicker;
    case "failed":
      return { step: "error", path: "path" in state ? state.path : null, kind: action.kind };
    case "cancel":
      return initialPicker;
  }
}

/** Shortens a long path in the middle, keeping its start and the final folder. */
export function middleTruncate(path: string, max: number): string {
  if (path.length <= max) return path;
  const last = path.split(/[\\/]/).pop() ?? "";
  const tail = Math.min(last.length, max - 12);
  const head = max - 1 - tail;
  return `${path.slice(0, head)}…${path.slice(path.length - tail)}`;
}
