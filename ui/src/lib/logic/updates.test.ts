import { describe, expect, it } from "vitest";
import {
  initialUpdate,
  notesBlocks,
  rateLimitReset,
  readLastChecked,
  reduceUpdate,
  relativeTime,
  tokenDraft,
  updateErrorCode,
  updateErrorKey,
  writeLastChecked,
  type UpdateInfo,
} from "./updates";

const info = (x: Partial<UpdateInfo> = {}): UpdateInfo => ({
  current: "0.3.2",
  latest: "0.3.3",
  isNewer: true,
  isPrerelease: true,
  name: "v0.3.3",
  publishedAt: "2026-10-01T10:00:00Z",
  notes: "",
  htmlUrl: "https://github.com/x/y/releases/tag/v0.3.3",
  asset: null,
  installKind: "rpm",
  manualInstallRequired: false,
  signed: true,
  checkedAt: "2026-10-18T10:00:00Z",
  ...x,
});

describe("reduceUpdate", () => {
  it("goes idle -> checking -> available or up to date", () => {
    let s = reduceUpdate(initialUpdate, { type: "check" });
    expect(s.step).toBe("checking");
    expect(reduceUpdate(s, { type: "check" })).toBe(s);
    expect(reduceUpdate(s, { type: "done", info: info() }).step).toBe("available");
    expect(reduceUpdate(s, { type: "done", info: info({ isNewer: false }) }).step).toBe("up_to_date");
    expect(reduceUpdate(s, { type: "done", info: info({ latest: null, isNewer: false }) }).step).toBe("up_to_date");
    s = reduceUpdate(s, { type: "failed", error: { code: "network" }, tokenSaved: false });
    expect(s).toMatchObject({ step: "error", code: "network" });
  });

  it("keeps the previous result out of the way while checking again", () => {
    const done = reduceUpdate(reduceUpdate(initialUpdate, { type: "check" }), { type: "done", info: info() });
    expect(reduceUpdate(done, { type: "check" }).step).toBe("checking");
  });

  it("carries the rate limit reset and whether a token was saved", () => {
    const s = reduceUpdate(initialUpdate, {
      type: "failed",
      error: { code: "rate_limited", message: "rate limit exceeded, resets at 2026-10-18T11:00:00Z" },
      tokenSaved: true,
    });
    expect(s).toMatchObject({ step: "error", code: "rate_limited", resetAt: "2026-10-18T11:00:00Z", tokenSaved: true });
  });
});

describe("error mapping", () => {
  it("reads the code from objects, JSON-less strings and unknowns", () => {
    expect(updateErrorCode({ code: "no_releases", message: "x" })).toBe("no_releases");
    expect(updateErrorCode({ code: "weird" })).toBe("internal");
    expect(updateErrorCode("boom")).toBe("internal");
    expect(updateErrorCode(null)).toBe("internal");
  });

  it("says something different for unauthorized with and without a token", () => {
    expect(updateErrorKey("update_unauthorized", false)).toBe("upd_err_private");
    expect(updateErrorKey("update_unauthorized", true)).toBe("upd_err_rejected");
    expect(updateErrorKey("invalid_token", false)).toBe("upd_err_invalid_token");
    for (const c of ["rate_limited", "network", "no_releases", "invalid_response", "internal"] as const) {
      expect(updateErrorKey(c, false)).toBe(`upd_err_${c}`);
    }
  });

  it("extracts an ISO reset time only when present", () => {
    expect(rateLimitReset("resets at 2026-10-18T11:00:00Z")).toBe("2026-10-18T11:00:00Z");
    expect(rateLimitReset("rate limit")).toBeNull();
    expect(rateLimitReset(undefined)).toBeNull();
  });
});

describe("relativeTime", () => {
  const now = Date.parse("2026-10-18T12:00:00Z");
  it("formats in es and en", () => {
    expect(relativeTime("2026-10-18T11:57:00Z", now, "es")).toBe("hace 3 min");
    expect(relativeTime("2026-10-18T11:57:00Z", now, "en")).toBe("3 min. ago");
    expect(relativeTime("2026-10-18T09:00:00Z", now, "es")).toBe("hace 3 h");
    expect(relativeTime("2026-10-15T12:00:00Z", now, "es")).toBe("hace 3 días");
  });
  it("says just now under a minute and survives bad input", () => {
    expect(relativeTime("2026-10-18T11:59:40Z", now, "es")).toBe("ahora mismo");
    expect(relativeTime("2026-10-18T11:59:40Z", now, "en")).toBe("just now");
    expect(relativeTime("nope", now, "es")).toBe("");
  });
});

describe("last checked storage", () => {
  it("round-trips and never throws", () => {
    const m = new Map<string, string>();
    const ok = { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
    expect(readLastChecked(ok)).toBeNull();
    writeLastChecked(ok, "2026-10-18T10:00:00Z");
    expect(readLastChecked(ok)).toBe("2026-10-18T10:00:00Z");
    const bad = {
      getItem: () => {
        throw new Error("x");
      },
      setItem: () => {
        throw new Error("x");
      },
    };
    expect(readLastChecked(bad)).toBeNull();
    expect(() => writeLastChecked(bad, "x")).not.toThrow();
  });
});

describe("tokenDraft", () => {
  it("trims, rejects empty and spaced tokens, and clears", () => {
    expect(tokenDraft("  ghp_abc  ")).toEqual({ value: "ghp_abc", valid: true });
    expect(tokenDraft("   ")).toEqual({ value: "", valid: false });
    expect(tokenDraft("ghp abc").valid).toBe(false);
    expect(tokenDraft("a".repeat(256)).valid).toBe(false);
  });
});

describe("notesBlocks", () => {
  it("turns light markdown into plain text blocks without any HTML", () => {
    const blocks = notesBlocks("## What's new\n\n- **Fast** thumbnails\n- see [docs](http://x) and `code`\n\nText <script>alert(1)</script> <b>x</b>");
    expect(blocks).toEqual([
      { kind: "heading", text: "What's new" },
      { kind: "item", text: "Fast thumbnails" },
      { kind: "item", text: "see docs and code" },
      { kind: "text", text: "Text <script>alert(1)</script> <b>x</b>" },
    ]);
  });
  it("is empty for blank notes", () => {
    expect(notesBlocks("  \n ")).toEqual([]);
  });
});
