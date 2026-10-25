import { afterEach, describe, expect, it, vi } from "vitest";
import { createMockApi } from "../mock/mockApi";
import { createGoldenlifeSessionAdapter, PlatformSession } from "./downloads.svelte";
import type { GoldenlifeCourse } from "../types";

afterEach(() => vi.unstubAllGlobals());

describe("Golden Life session adapter", () => {
  it("hydrates status, rejects bad credentials, logs in, retains broken rows, and clears on logout", async () => {
    vi.stubGlobal("location", { search: "" });
    const api = createMockApi();
    const session = new PlatformSession<GoldenlifeCourse>();
    await session.init(createGoldenlifeSessionAdapter(api));
    expect(session.session).toBe("disconnected");
    expect(await session.loginManual(async () => (await api.goldenlifeLoginCredentials("ana@example.com", "wrong")).courses)).toBe(false);
    expect(session.sessionError?.code).toBe("invalid_credentials");
    expect(await session.loginManual(async () => (await api.goldenlifeLoginCredentials("ana@example.com", "synthetic")).courses)).toBe(true);
    expect(session.session).toBe("connected");
    expect(session.items.some((course) => course.id === null && course.issue)).toBe(true);
    await session.logout();
    expect(session.session).toBe("disconnected");
    expect(session.items).toEqual([]);
  });
});
