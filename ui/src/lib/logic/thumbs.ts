// Thumbnail requests from poster cards: dedupe, batch, cancel what left the
// screen before being processed, resolve ready events, and keep an
// in-memory cache so remounted cards show their image at once. No I/O.

export interface ThumbItem {
  path: string;
  atSecs: number;
  /** Requested image width; requests are batched per width. Not part of the identity. */
  width?: number;
}

export type ThumbResult =
  | (ThumbItem & { state: "ready"; url: string })
  | (ThumbItem & { state: "pending" })
  | (ThumbItem & { state: "unavailable"; reason?: string });

/** Event payload of thumbnails://ready. */
export type ThumbReady = (ThumbItem & { url: string; state?: "ready" }) | (ThumbItem & { state: "unavailable"; reason?: string });

export type ThumbLookup = { state: "ready"; url: string } | { state: "pending" } | { state: "unavailable" } | { state: "idle" };

export const thumbKey = (i: ThumbItem): string => `${i.atSecs}|${i.path}`;

export class ThumbQueue {
  #ready = new Map<string, { path: string; url: string }>();
  /** Invalidated images: still shown until the re-request answers. */
  #stale = new Map<string, { path: string; url: string }>();
  #unavailable = new Map<string, string>();
  /** Requested (sent) and not resolved yet. */
  #pending = new Map<string, ThumbItem>();
  /** Wanted by visible cards and not sent yet. */
  #toSend = new Map<string, ThumbItem>();
  /** How many visible cards want each key. */
  #wanted = new Map<string, number>();
  #cancels = new Set<string>();

  want(item: ThumbItem): void {
    const k = thumbKey(item);
    this.#wanted.set(k, (this.#wanted.get(k) ?? 0) + 1);
    this.#cancels.delete(item.path);
    if (this.#ready.has(k) || this.#unavailable.has(k) || this.#pending.has(k)) return;
    this.#toSend.set(k, item);
  }

  leave(item: ThumbItem): void {
    const k = thumbKey(item);
    const n = (this.#wanted.get(k) ?? 0) - 1;
    if (n > 0) {
      this.#wanted.set(k, n);
      return;
    }
    this.#wanted.delete(k);
    this.#toSend.delete(k);
    if (this.#pending.delete(k)) this.#cancels.add(item.path);
  }

  takeBatch(max: number): ThumbItem[] {
    const first = this.#toSend.values().next().value;
    const batch = [...this.#toSend.entries()].filter(([, i]) => i.width === first?.width).slice(0, max);
    for (const [k, item] of batch) {
      this.#toSend.delete(k);
      this.#pending.set(k, item);
    }
    return batch.map(([, item]) => item);
  }

  takeCancels(): string[] {
    const out = [...this.#cancels];
    this.#cancels.clear();
    return out;
  }

  applyResults(results: readonly ThumbResult[]): void {
    for (const r of results) {
      if (r.state === "pending") continue;
      this.#resolve(r, r.state === "ready" ? r.url : null);
    }
  }

  applyReady(e: ThumbReady): void {
    this.#resolve(e, e.state === "unavailable" ? null : e.url);
  }

  lookup(item: ThumbItem): ThumbLookup {
    const k = thumbKey(item);
    const hit = this.#ready.get(k) ?? this.#stale.get(k);
    if (hit) return { state: "ready", url: hit.url };
    if (this.#unavailable.has(k)) return { state: "unavailable" };
    if (this.#pending.has(k) || this.#toSend.has(k)) return { state: "pending" };
    return { state: "idle" };
  }

  #resolve(item: ThumbItem, url: string | null): void {
    const k = thumbKey(item);
    this.#pending.delete(k);
    this.#toSend.delete(k);
    this.#stale.delete(k);
    this.#ready.delete(k);
    this.#unavailable.delete(k);
    if (url) this.#ready.set(k, { path: item.path, url });
    else this.#unavailable.set(k, item.path);
  }

  /** The files behind these paths (all when omitted) may have changed: keep
   * showing what we have, forget failures and in-flight work, and let the
   * next want() revalidate. */
  invalidate(paths?: readonly string[]): void {
    const hit = (p: string) => !paths || paths.includes(p);
    for (const [k, v] of this.#ready) {
      if (!hit(v.path)) continue;
      this.#ready.delete(k);
      this.#stale.set(k, v);
    }
    for (const [k, p] of this.#unavailable) if (hit(p)) this.#unavailable.delete(k);
    for (const [k, i] of this.#pending) if (hit(i.path)) this.#pending.delete(k);
  }

  /** The image failed to load (file gone or unreadable): fall back to the poster for good. */
  fail(item: ThumbItem): void {
    this.#resolve(item, null);
  }
}

/** Hero still: the thumbnail when ready; the video only as a fallback once thumbnails are unavailable. */
export function heroSource(l: ThumbLookup): "thumb" | "video" | "poster" {
  if (l.state === "ready") return "thumb";
  return l.state === "unavailable" ? "video" : "poster";
}

const DISMISS_KEY = "lumen.ffmpegNoteDismissed";
interface KV {
  getItem(k: string): string | null;
  setItem(k: string, v: string): void;
}

export function readDismissed(store: KV): boolean {
  try {
    return store.getItem(DISMISS_KEY) === "1";
  } catch {
    return false;
  }
}

export function writeDismissed(store: KV): void {
  try {
    store.setItem(DISMISS_KEY, "1");
  } catch {
    // Storage blocked: the note simply reappears next time.
  }
}
