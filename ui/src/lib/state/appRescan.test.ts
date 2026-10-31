import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { getApi } from "../api";
import { app } from "./app.svelte";

beforeEach(() => {
  vi.stubGlobal("location", { search: "" });
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("rescanSoon", () => {
  it("rescans the library at once, then at most once per interval however many lessons land", async () => {
    const api = await getApi();
    const scan = vi.spyOn(api, "scanLibrary");
    await app.init();
    scan.mockClear();
    vi.useFakeTimers();
    app.rescanSoon();
    app.rescanSoon();
    app.rescanSoon();
    await vi.advanceTimersByTimeAsync(0);
    expect(scan).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(7000);
    expect(scan).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1500);
    expect(scan).toHaveBeenCalledTimes(2);
  });
});
