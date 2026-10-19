// Download progress state, driven by command results and backend events.
// Keyed by download id (unique across platforms): events carry the group
// display name, while start and re-attach carry the platform key (Skool slug
// or Hotmart course id), so both are kept.
import type { PlatformId, ProgressEvent, ReportEvent, RunningDownload } from "../types";

export interface DownloadEntry {
  id: string;
  platform: PlatformId;
  key: string | null;
  name: string | null;
  course: string;
  lesson: string;
  percent: number;
  completed: number;
  total: number;
  status: "running" | "finished";
  report: ReportEvent | null;
}

export type DownloadsState = Readonly<Record<string, DownloadEntry>>;

export type DownloadAction =
  | { type: "started"; id: string; platform: PlatformId; key: string }
  | { type: "attached"; running: RunningDownload[] }
  | { type: "progress"; event: ProgressEvent }
  | { type: "report"; event: ReportEvent }
  | { type: "dismiss"; id: string };

export const initialDownloads: DownloadsState = {};

const fresh = (id: string, platform: PlatformId, key: string | null): DownloadEntry => ({
  id,
  platform,
  key,
  name: null,
  course: "",
  lesson: "",
  percent: 0,
  completed: 0,
  total: 0,
  status: "running",
  report: null,
});

export function reduceDownloads(state: DownloadsState, action: DownloadAction): DownloadsState {
  switch (action.type) {
    case "started":
      return { ...state, [action.id]: { ...fresh(action.id, action.platform, action.key), ...pick(state[action.id]) } };
    case "attached": {
      const next = { ...state };
      for (const r of action.running) {
        const prev = next[r.id];
        next[r.id] = prev
          ? { ...prev, platform: r.platform, key: r.group, status: "running" }
          : fresh(r.id, r.platform, r.group);
      }
      return next;
    }
    case "progress": {
      const e = action.event;
      const prev = state[e.id] ?? fresh(e.id, e.platform, null);
      if (prev.status === "finished" || e.completed < prev.completed) return state;
      return {
        ...state,
        [e.id]: {
          ...prev,
          name: e.group,
          course: e.course,
          lesson: e.lesson,
          percent: e.percent,
          completed: e.completed,
          total: e.total,
        },
      };
    }
    case "report": {
      const e = action.event;
      const prev = state[e.id] ?? fresh(e.id, e.platform, null);
      return {
        ...state,
        [e.id]: {
          ...prev,
          name: prev.name ?? e.group,
          status: "finished",
          completed: e.completedLessons,
          total: e.totalLessons,
          report: e,
        },
      };
    }
    case "dismiss": {
      const next = { ...state };
      delete next[action.id];
      return next;
    }
  }
}

/** Progress already known for an id that is (re)started keeps its numbers. */
function pick(prev: DownloadEntry | undefined): Partial<DownloadEntry> {
  return prev && prev.status === "running" ? { completed: prev.completed, total: prev.total, name: prev.name } : {};
}

/** Latest download for an item of a platform, preferring a running one. */
export function downloadFor(
  state: DownloadsState,
  item: { platform: PlatformId; key: string; name: string },
): DownloadEntry | undefined {
  const mine = Object.values(state).filter(
    (d) => d.platform === item.platform && (d.key === item.key || (d.key === null && d.name === item.name)),
  );
  const byId = (a: DownloadEntry, b: DownloadEntry) => idNumber(b.id) - idNumber(a.id);
  return mine.filter((d) => d.status === "running").sort(byId)[0] ?? mine.sort(byId)[0];
}

const idNumber = (id: string): number => Number(id.replace(/\D+/g, "")) || 0;

export function overallPercent(d: DownloadEntry): number {
  if (!d.total) return 0;
  const done = Math.min(d.total, d.completed + Math.max(0, Math.min(100, d.percent)) / 100);
  return Math.round((done / d.total) * 100);
}
