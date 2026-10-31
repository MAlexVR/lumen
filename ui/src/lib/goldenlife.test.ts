import { afterEach, describe, expect, it, vi } from "vitest";
import { createMockApi } from "./mock/mockApi";
import { createGoldenlifeCommands } from "./api";

afterEach(() => vi.unstubAllGlobals());

describe("Golden Life preview adapter", () => {
  it("logs in with credentials, lists shelf-tagged courses, downloads and cancels", async () => {
    vi.stubGlobal("location", { search: "" });
    const api = createMockApi();
    expect(await api.goldenlifeSessionStatus()).toEqual({ loggedIn: false });
    await expect(api.goldenlifeLoginCredentials("ana@example.com", "wrong")).rejects.toMatchObject({ code: "invalid_credentials" });
    const result = await api.goldenlifeLoginCredentials("ana@example.com", "synthetic");
    expect(result.loggedIn).toBe(true);
    expect(result.courses.some((c) => c.shelf)).toBe(true);
    expect(result.courses.some((c) => c.id === null && c.issue)).toBe(true);
    const library = await api.scanLibrary("/mock-library");
    expect(library.platforms.find((p) => p.id === "goldenlife")?.groups[0].id).toBe("goldenlife/Creator Basics");
    const id = await api.goldenlifeDownloadStart("101");
    expect(await api.goldenlifeDownloadsRunning()).toEqual([{ id, platform: "goldenlife", group: "101", paused: false }]);
    expect(await api.goldenlifeDownloadCancel(id)).toBe(true);
    await api.goldenlifeLogout();
    expect(await api.goldenlifeSessionStatus()).toEqual({ loggedIn: false });
  });

  it("emits a platform-qualified report for the preview scenario", async () => {
    vi.stubGlobal("location", { search: "?goldenlife=report-ok" });
    const api = createMockApi();
    const reportEvent = new Promise<import("./types").ReportEvent>((resolve) => {
      void api.onDownloadReport(resolve);
    });
    const event = await Promise.race([
      reportEvent,
      new Promise<never>((_, reject) => setTimeout(() => reject(new Error("report event timed out")), 1500)),
    ]);
    expect(event).toMatchObject({ platform: "goldenlife", complete: true });
  });

  it("maps Golden Life commands to the backend invoke contract", async () => {
    const calls: Array<[string, Record<string, unknown> | undefined]> = [];
    const invoke = async <T>(command: string, args?: Record<string, unknown>): Promise<T> => {
      calls.push([command, args]);
      return undefined as T;
    };
    const commands = createGoldenlifeCommands(invoke);
    await commands.goldenlifeLoginCredentials("ana@example.com", "pw");
    await commands.goldenlifeSessionStatus();
    await commands.goldenlifeLogout();
    await commands.goldenlifeCourses();
    await commands.goldenlifeDownloadStart("101");
    await commands.goldenlifeDownloadCancel("dl-1");
    await commands.goldenlifeDownloadsRunning();
    expect(calls).toEqual([
      ["goldenlife_login_credentials", { email: "ana@example.com", password: "pw" }],
      ["goldenlife_session_status", undefined],
      ["goldenlife_logout", undefined],
      ["goldenlife_courses", undefined],
      ["goldenlife_download_start", { course: "101" }],
      ["goldenlife_download_cancel", { id: "dl-1" }],
      ["goldenlife_downloads_running", undefined],
    ]);
  });
});
