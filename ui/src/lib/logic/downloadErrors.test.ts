import { describe, expect, it } from "vitest";
import { ERROR_CODES, toDownloadError } from "./downloadErrors";

describe("toDownloadError", () => {
  it("keeps a typed backend error", () => {
    expect(toDownloadError({ code: "session_expired", message: "expired" })).toEqual({
      code: "session_expired",
      message: "expired",
    });
  });

  it.each(ERROR_CODES.map((c) => [c]))("accepts the %s code", (code) => {
    expect(toDownloadError({ code, message: "m" }).code).toBe(code);
  });

  it("maps unknown codes, strings and Error objects to internal", () => {
    expect(toDownloadError({ code: "weird", message: "x" })).toEqual({ code: "internal", message: "x" });
    expect(toDownloadError("boom")).toEqual({ code: "internal", message: "boom" });
    expect(toDownloadError(new Error("bad"))).toEqual({ code: "internal", message: "bad" });
    expect(toDownloadError(undefined)).toEqual({ code: "internal", message: "" });
  });

  it("knows Hotmart's invalid_token code", () => {
    expect(toDownloadError({ code: "invalid_token", message: "bad token" })).toEqual({ code: "invalid_token", message: "bad token" });
  });

  it.each(["invalid_url", "unsupported_url", "unavailable", "ytdlp_missing"])("knows YouTube's %s code", (code) => {
    expect(toDownloadError({ code, message: "m" }).code).toBe(code);
  });

  it.each(["bot_challenge", "diagnose_running", "drm_protected"])("knows Udemy's %s code", (code) => {
    expect(toDownloadError({ code, message: "m" }).code).toBe(code);
  });

  it("recognizes a JSON string error from the bridge", () => {
    expect(toDownloadError('{"code":"network","message":"offline"}')).toEqual({ code: "network", message: "offline" });
  });
});
