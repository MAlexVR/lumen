import { describe, expect, it } from "vitest";
import type { YoutubeItem, YoutubePreview } from "../types";
import {
  canonicalUrl,
  initialSelection,
  loadQuality,
  reduceSelection,
  saveQuality,
  selectionTotals,
  youtubeHint,
} from "./youtube";

describe("youtubeHint", () => {
  it.each([
    ["https://www.youtube.com/watch?v=dQw4w9WgXcQ", "video"],
    ["youtube.com/watch?v=dQw4w9WgXcQ&t=42s", "video"],
    ["https://youtu.be/dQw4w9WgXcQ", "video"],
    ["https://m.youtube.com/shorts/dQw4w9WgXcQ", "video"],
    ["https://music.youtube.com/watch?v=dQw4w9WgXcQ", "video"],
    ["https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=PLx", "video"],
    ["https://www.youtube.com/playlist?list=PL590L5WQmH8fJ54F369BLDSqIwcs-TCfs", "playlist"],
    ["https://www.youtube.com/watch?list=PL590L5WQmH8fJ54F369BLDSqIwcs-TCfs", "playlist"],
  ])("recognises %j as a %s", (url, kind) => {
    expect(youtubeHint(url)).toEqual({ state: "valid", kind });
  });

  it("flags empty input, non-links, other sites and unsupported pages", () => {
    expect(youtubeHint("  ")).toEqual({ state: "empty", kind: null });
    expect(youtubeHint("hola que tal")).toEqual({ state: "not_url", kind: null });
    expect(youtubeHint("https://vimeo.com/123")).toEqual({ state: "other_site", kind: null });
    expect(youtubeHint("https://www.youtube.com/@canal")).toEqual({ state: "unsupported", kind: null });
    expect(youtubeHint("https://www.youtube.com/watch?v=short")).toEqual({ state: "unsupported", kind: null });
  });
});

const item = (id: string, durationSecs: number | null, available = true): YoutubeItem => ({
  id,
  title: id,
  durationSecs,
  thumbnailUrl: null,
  available,
  reason: available ? null : "private video",
});
const items = [item("a", 60), item("b", 120), item("x", null, false), item("c", null)];

describe("selection", () => {
  it("starts with every available video selected", () => {
    expect(initialSelection(items)).toEqual(["a", "b", "c"]);
  });

  it("toggles available items only", () => {
    let s = initialSelection(items);
    s = reduceSelection(s, { type: "toggle", id: "b" }, items);
    expect(s).toEqual(["a", "c"]);
    s = reduceSelection(s, { type: "toggle", id: "b" }, items);
    expect(s).toEqual(["a", "b", "c"]);
    expect(reduceSelection(s, { type: "toggle", id: "x" }, items)).toEqual(["a", "b", "c"]);
  });

  it("selects all available or none, keeping playlist order", () => {
    expect(reduceSelection([], { type: "all" }, items)).toEqual(["a", "b", "c"]);
    expect(reduceSelection(["a"], { type: "none" }, items)).toEqual([]);
  });

  it("totals the selected count and known duration", () => {
    expect(selectionTotals(items, ["a", "b", "c"])).toEqual({ count: 3, available: 3, seconds: 180, unknown: 1 });
    expect(selectionTotals(items, [])).toEqual({ count: 0, available: 3, seconds: 0, unknown: 0 });
  });
});

describe("quality persistence", () => {
  const store = () => {
    const m = new Map<string, string>();
    return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
  };

  it("defaults to best ≤1080p and remembers a valid choice", () => {
    const s = store();
    expect(loadQuality(s)).toBe("best1080");
    saveQuality(s, "audio");
    expect(loadQuality(s)).toBe("audio");
  });

  it("ignores invalid stored values and broken storage", () => {
    const s = store();
    s.setItem("lumen.youtubeQuality", "4k");
    expect(loadQuality(s)).toBe("best1080");
    const broken = {
      getItem: () => {
        throw new Error("denied");
      },
      setItem: () => {
        throw new Error("denied");
      },
    };
    expect(loadQuality(broken)).toBe("best1080");
    expect(() => saveQuality(broken, "720")).not.toThrow();
  });
});

describe("canonicalUrl", () => {
  const preview = (kind: "video" | "playlist", id: string): YoutubePreview => ({
    kind,
    id,
    title: "t",
    channel: "c",
    thumbnailUrl: null,
    durationSecs: null,
    folder: "f",
  });

  it("matches the backend's group key", () => {
    expect(canonicalUrl(preview("video", "dQw4w9WgXcQ"))).toBe("https://www.youtube.com/watch?v=dQw4w9WgXcQ");
    expect(canonicalUrl(preview("playlist", "PLabc"))).toBe("https://www.youtube.com/playlist?list=PLabc");
  });
});
