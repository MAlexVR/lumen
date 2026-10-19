import { afterEach, describe, expect, it, vi } from "vitest";
import { createMockApi } from "../mock/mockApi";
import { createPluralsightSessionAdapter, PlatformSession } from "./downloads.svelte";
import type { PluralsightCourse } from "../types";

afterEach(() => vi.unstubAllGlobals());

describe("Pluralsight session adapter", () => {
  it("hydrates status, logs in, retains invalid rows, and clears on logout", async () => {
    vi.stubGlobal("location", { search: "" });
    const api = createMockApi();
    const session = new PlatformSession<PluralsightCourse>();
    await session.init(createPluralsightSessionAdapter(api));
    expect(session.session).toBe("disconnected");
    expect(await session.loginManual(async () => (await api.pluralsightLoginCredentials("session=synthetic")).courses)).toBe(true);
    expect(session.session).toBe("connected");
    expect(session.items.some((course) => course.id === null && course.issue)).toBe(true);
    await session.logout();
    expect(session.session).toBe("disconnected");
    expect(session.items).toEqual([]);
  });
});
