import { afterEach, describe, expect, it, vi } from "vitest";
import { createMockApi } from "../mock/mockApi";
import { center, createGoldenlifeSessionAdapter, PlatformSession } from "./downloads.svelte";
import type { GoldenlifeCourse } from "../types";

afterEach(() => vi.unstubAllGlobals());

describe("pausing a download", () => {
  it("pauses and resumes through the backend and tracks it per download id, then cancel still works", async () => {
    vi.stubGlobal("location", { search: "" });
    const api = createMockApi();
    center.attach(api);
    const session = new PlatformSession<GoldenlifeCourse>();
    await session.init(createGoldenlifeSessionAdapter(api));
    await session.loginManual(async () => (await api.goldenlifeLoginCredentials("ana@example.com", "synthetic")).courses);
    await session.start("101");
    const id = Object.keys(center.downloads).at(-1) as string;
    expect(center.downloads[id].paused).toBe(false);

    await session.pause(id);
    expect(center.downloads[id].paused).toBe(true);
    expect((await api.goldenlifeDownloadsRunning())[0].paused).toBe(true);

    await session.resume(id);
    expect(center.downloads[id].paused).toBe(false);

    await session.pause(id);
    await session.cancel(id);
    expect(session.cancelling[id]).toBe(true);
  });

  it("leaves the state alone when the backend does not know the download", async () => {
    vi.stubGlobal("location", { search: "" });
    const api = createMockApi();
    center.attach(api);
    await center.setPaused("dl-missing", true);
    expect(center.downloads["dl-missing"]).toBeUndefined();
  });
});
