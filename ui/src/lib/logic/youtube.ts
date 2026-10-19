// YouTube screen logic: a client-side link hint (the backend validates for
// real), the playlist selection, totals, the remembered quality and the
// canonical link used as download group key.
import type { YoutubeItem, YoutubePreview, YoutubeQuality } from "../types";

export type HintState = "empty" | "valid" | "not_url" | "other_site" | "unsupported";

const HOSTS = new Set(["youtube.com", "www.youtube.com", "m.youtube.com", "music.youtube.com", "youtu.be", "www.youtube-nocookie.com", "youtube-nocookie.com"]);
const VIDEO_ID = /^[A-Za-z0-9_-]{11}$/;
const LIST_ID = /^[A-Za-z0-9_-]{2,}$/;

export function youtubeHint(input: string): { state: HintState; kind: "video" | "playlist" | null } {
  const text = input.trim();
  if (!text) return { state: "empty", kind: null };
  let url: URL;
  try {
    url = new URL(/^[a-z]+:\/\//i.test(text) ? text : `https://${text}`);
    if (!url.hostname.includes(".") || /\s/.test(text)) throw new Error("not a link");
  } catch {
    return { state: "not_url", kind: null };
  }
  const host = url.hostname.toLowerCase();
  if (!HOSTS.has(host)) return { state: "other_site", kind: null };
  const segs = url.pathname.split("/").filter(Boolean);
  const v = url.searchParams.get("v");
  const list = url.searchParams.get("list");
  let video: string | null = null;
  if (host === "youtu.be") video = segs[0] ?? null;
  else if (segs[0] === "watch") video = v;
  else if (["shorts", "embed", "live"].includes(segs[0] ?? "")) video = segs[1] ?? null;
  if (video !== null) return VIDEO_ID.test(video) ? { state: "valid", kind: "video" } : { state: "unsupported", kind: null };
  if ((segs[0] === "playlist" || segs[0] === "watch") && list && LIST_ID.test(list)) return { state: "valid", kind: "playlist" };
  return { state: "unsupported", kind: null };
}

export function initialSelection(items: readonly YoutubeItem[]): string[] {
  return items.filter((i) => i.available).map((i) => i.id);
}

export type SelectionAction = { type: "toggle"; id: string } | { type: "all" } | { type: "none" };

/** Selected ids in playlist order; unavailable items can never be selected. */
export function reduceSelection(selected: readonly string[], action: SelectionAction, items: readonly YoutubeItem[]): string[] {
  const set = new Set(selected);
  if (action.type === "all") return initialSelection(items);
  if (action.type === "none") return [];
  const item = items.find((i) => i.id === action.id);
  if (!item?.available) return [...selected];
  if (set.has(action.id)) set.delete(action.id);
  else set.add(action.id);
  return items.filter((i) => set.has(i.id)).map((i) => i.id);
}

export function selectionTotals(items: readonly YoutubeItem[], selected: readonly string[]) {
  const chosen = items.filter((i) => selected.includes(i.id));
  return {
    count: chosen.length,
    available: items.filter((i) => i.available).length,
    seconds: chosen.reduce((n, i) => n + (i.durationSecs ?? 0), 0),
    unknown: chosen.filter((i) => i.durationSecs === null).length,
  };
}

const QUALITY_KEY = "lumen.youtubeQuality";
export const QUALITIES: readonly YoutubeQuality[] = ["best1080", "720", "audio"];

interface KeyValue {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

export function loadQuality(store: KeyValue): YoutubeQuality {
  try {
    const v = store.getItem(QUALITY_KEY);
    return QUALITIES.includes(v as YoutubeQuality) ? (v as YoutubeQuality) : "best1080";
  } catch {
    return "best1080";
  }
}

export function saveQuality(store: KeyValue, quality: YoutubeQuality): void {
  try {
    store.setItem(QUALITY_KEY, quality);
  } catch {
    /* not remembered */
  }
}

/** Same key the backend uses for running downloads and reports. */
export const canonicalUrl = (p: Pick<YoutubePreview, "kind" | "id">): string =>
  p.kind === "video" ? `https://www.youtube.com/watch?v=${p.id}` : `https://www.youtube.com/playlist?list=${p.id}`;
