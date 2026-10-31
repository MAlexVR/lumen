import { afterEach, describe, expect, it, vi } from "vitest";
import { createMockApi } from "../mock/mockApi";
import { createHarbizSessionAdapter, PlatformSession } from "./downloads.svelte";
import type { HarbizCourse } from "../types";

afterEach(() => vi.unstubAllGlobals());

describe("Harbiz session adapter", () => {
  it("hydrates status, rejects bad credentials, logs in, lists the agenda, and clears on logout", async () => {
    vi.stubGlobal("location", { search: "" });
    const api = createMockApi();
    const session = new PlatformSession<HarbizCourse>();
    await session.init(createHarbizSessionAdapter(api));
    expect(session.session).toBe("disconnected");
    expect(await session.loginManual(async () => (await api.harbizLoginCredentials("ana@example.com", "wrong")).courses)).toBe(false);
    expect(session.sessionError?.code).toBe("invalid_credentials");
    expect(await session.loginManual(async () => (await api.harbizLoginCredentials("ana@example.com", "synthetic")).courses)).toBe(true);
    expect(session.session).toBe("connected");
    expect(session.items[0]).toMatchObject({ id: "harbiz", pending: 4 });
    await session.logout();
    expect(session.session).toBe("disconnected");
    expect(session.items).toEqual([]);
  });
});
