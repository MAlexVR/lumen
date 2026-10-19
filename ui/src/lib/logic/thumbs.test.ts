import { describe, expect, it } from "vitest";
import { ThumbQueue, thumbKey, heroSource, readDismissed, writeDismissed } from "./thumbs";

const it1 = { path: "Skool/G/01 - A/v.mp4", atSecs: 8 };
const it2 = { path: "Skool/G/02 - B/v.mp4", atSecs: 95 };

describe("ThumbQueue", () => {
  it("batches wanted items once, up to the max, and dedupes", () => {
    const q = new ThumbQueue();
    q.want(it1);
    q.want(it1);
    q.want(it2);
    expect(q.takeBatch(1)).toEqual([it1]);
    expect(q.takeBatch(24)).toEqual([it2]);
    expect(q.takeBatch(24)).toEqual([]);
  });

  it("applies immediate results: ready is cached, pending waits, unavailable is final", () => {
    const q = new ThumbQueue();
    q.want(it1);
    q.want(it2);
    q.takeBatch(24);
    q.applyResults([
      { ...it1, state: "ready", url: "http://t/1.jpg" },
      { ...it2, state: "pending" },
    ]);
    expect(q.lookup(it1)).toEqual({ state: "ready", url: "http://t/1.jpg" });
    expect(q.lookup(it2)).toEqual({ state: "pending" });
    q.applyReady({ ...it2, state: "unavailable", reason: "ffmpeg_failed" });
    expect(q.lookup(it2)).toEqual({ state: "unavailable" });
    q.want(it2);
    expect(q.takeBatch(24)).toEqual([]);
  });

  it("resolves a ready event by path and moment", () => {
    const q = new ThumbQueue();
    q.want(it2);
    q.takeBatch(24);
    q.applyResults([{ ...it2, state: "pending" }]);
    q.applyReady({ path: it2.path, atSecs: 95, url: "http://t/2.jpg" });
    expect(q.lookup(it2)).toEqual({ state: "ready", url: "http://t/2.jpg" });
  });

  it("serves cached urls instantly to a remounted card without a new request", () => {
    const q = new ThumbQueue();
    q.want(it1);
    q.takeBatch(24);
    q.applyResults([{ ...it1, state: "ready", url: "u" }]);
    q.want(it1);
    expect(q.takeBatch(24)).toEqual([]);
    expect(q.lookup(it1)).toEqual({ state: "ready", url: "u" });
  });

  it("drops unsent items that left the screen and lists sent pending ones to cancel", () => {
    const q = new ThumbQueue();
    q.want(it1);
    q.want(it2);
    q.leave(it1);
    expect(q.takeBatch(24)).toEqual([it2]);
    q.applyResults([{ ...it2, state: "pending" }]);
    q.leave(it2);
    expect(q.takeCancels()).toEqual([it2.path]);
    expect(q.takeCancels()).toEqual([]);
    expect(q.lookup(it2)).toEqual({ state: "idle" });
    q.want(it2);
    expect(q.takeBatch(24)).toEqual([it2]);
  });

  it("does not cancel an item another card still wants", () => {
    const q = new ThumbQueue();
    q.want(it1);
    q.want(it1);
    q.takeBatch(24);
    q.applyResults([{ ...it1, state: "pending" }]);
    q.leave(it1);
    expect(q.takeCancels()).toEqual([]);
  });

  it("keys by path and moment", () => {
    expect(thumbKey(it1)).not.toBe(thumbKey({ ...it1, atSecs: 9 }));
  });
});

describe("heroSource", () => {
  it("uses the image when ready, the video only once thumbnails are unavailable, else the poster", () => {
    expect(heroSource({ state: "ready", url: "u" })).toBe("thumb");
    expect(heroSource({ state: "unavailable" })).toBe("video");
    expect(heroSource({ state: "pending" })).toBe("poster");
    expect(heroSource({ state: "idle" })).toBe("poster");
  });
});

describe("dismissal", () => {
  it("is remembered only when storage works", () => {
    const store = new Map<string, string>();
    const ok = { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v) };
    expect(readDismissed(ok)).toBe(false);
    writeDismissed(ok);
    expect(readDismissed(ok)).toBe(true);
    const broken = {
      getItem: () => {
        throw new Error("blocked");
      },
      setItem: () => {
        throw new Error("blocked");
      },
    };
    expect(readDismissed(broken)).toBe(false);
    expect(() => writeDismissed(broken)).not.toThrow();
  });
});

describe("ThumbQueue refresh", () => {
  const ready = (q: ThumbQueue, it: { path: string; atSecs: number }, url: string) => {
    q.want(it);
    q.takeBatch(24);
    q.applyResults([{ ...it, state: "ready", url }]);
  };

  it("revalidates everything while still showing the old image", () => {
    const q = new ThumbQueue();
    ready(q, it1, "old");
    q.invalidate();
    expect(q.lookup(it1)).toEqual({ state: "ready", url: "old" });
    q.want(it1);
    expect(q.takeBatch(24)).toEqual([it1]);
    q.applyResults([{ ...it1, state: "ready", url: "new" }]);
    expect(q.lookup(it1)).toEqual({ state: "ready", url: "new" });
  });

  it("falls back to the poster when a revalidated video is gone", () => {
    const q = new ThumbQueue();
    ready(q, it1, "old");
    q.invalidate();
    q.want(it1);
    q.takeBatch(24);
    q.applyResults([{ ...it1, state: "unavailable", reason: "invalid_path" }]);
    expect(q.lookup(it1)).toEqual({ state: "unavailable" });
  });

  it("invalidates only the affected paths and retries unavailable ones", () => {
    const q = new ThumbQueue();
    ready(q, it1, "u1");
    ready(q, it2, "u2");
    q.applyResults([{ path: "x.mp4", atSecs: 8, state: "unavailable" }]);
    q.invalidate([it1.path, "x.mp4"]);
    q.want(it1);
    q.want(it2);
    q.want({ path: "x.mp4", atSecs: 8 });
    expect(q.takeBatch(24).map((i) => i.path)).toEqual([it1.path, "x.mp4"]);
  });

  it("re-requests an in-flight item after invalidation", () => {
    const q = new ThumbQueue();
    q.want(it1);
    q.takeBatch(24);
    q.invalidate();
    q.want(it1);
    expect(q.takeBatch(24)).toEqual([it1]);
  });

  it("a broken image drops its entry and never loops", () => {
    const q = new ThumbQueue();
    ready(q, it1, "u");
    q.fail(it1);
    expect(q.lookup(it1)).toEqual({ state: "unavailable" });
    q.want(it1);
    expect(q.takeBatch(24)).toEqual([]);
  });

  it("applies a ready event that arrives after the card left", () => {
    const q = new ThumbQueue();
    q.want(it1);
    q.takeBatch(24);
    q.applyResults([{ ...it1, state: "pending" }]);
    q.leave(it1);
    q.applyReady({ ...it1, url: "late" });
    q.want(it1);
    expect(q.lookup(it1)).toEqual({ state: "ready", url: "late" });
    expect(q.takeBatch(24)).toEqual([]);
  });

  it("batches one width at a time", () => {
    const q = new ThumbQueue();
    q.want({ ...it1, width: 640 });
    q.want(it2);
    expect(q.takeBatch(24)).toEqual([{ ...it1, width: 640 }]);
    expect(q.takeBatch(24)).toEqual([it2]);
  });
});
