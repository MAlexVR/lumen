import { afterEach, describe, expect, it, vi } from "vitest";
import { createMockApi } from "./mock/mockApi";
import { createPluralsightCommands } from "./api";

afterEach(() => vi.unstubAllGlobals());

describe("Pluralsight preview adapter", () => {
  it("starts disconnected and exposes synthetic history courses after cookie login", async () => {
    vi.stubGlobal("location", { search: "" });
    const api = createMockApi();
    expect(await api.pluralsightSessionStatus()).toEqual({ loggedIn: false });
    const result = await api.pluralsightLoginCredentials("session=synthetic; XSRF-TOKEN=synthetic");
    expect(result.loggedIn).toBe(true);
    expect(result.courses.length).toBeGreaterThan(0);
    expect(result.courses.some((course) => course.id === null && course.issue)).toBe(true);
    expect(await api.pluralsightSessionStatus()).toEqual({ loggedIn: true });
    const library = await api.scanLibrary("/mock-library");
    expect(library.platforms.find((platform) => platform.id === "pluralsight")?.groups[0].id)
      .toBe("pluralsight/JavaScript Fundamentals-0123456789abcdef");
    const id = await api.pluralsightDownloadStart("course-js-101");
    expect(await api.pluralsightDownloadsRunning()).toEqual([{ id, platform: "pluralsight", group: "course-js-101" }]);
    expect(await api.pluralsightDownloadCancel(id)).toBe(true);
    await api.pluralsightLogout();
    expect(await api.pluralsightSessionStatus()).toEqual({ loggedIn: false });
  });


  it("rejects an invalid cookie paste without changing mock session status", async () => {
    vi.stubGlobal("location", { search: "" });
    const api = createMockApi();
    await expect(api.pluralsightLoginCredentials("analytics=synthetic")).rejects.toMatchObject({ code: "invalid_cookies" });
    expect(await api.pluralsightSessionStatus()).toEqual({ loggedIn: false });
  });

  it("emits a platform-qualified report for the Pluralsight preview scenario", async () => {
    vi.stubGlobal("location", { search: "?pluralsight=report-ok" });
    const api = createMockApi();
    const reportEvent = new Promise<import("./types").ReportEvent>((resolve) => {
      void api.onDownloadReport(resolve);
    });
    const event = await Promise.race([
      reportEvent,
      new Promise<never>((_, reject) => setTimeout(() => reject(new Error("report event timed out")), 1500)),
    ]);
    expect(event).toMatchObject({ platform: "pluralsight", group: "JavaScript Fundamentals", complete: true });
    expect(event.id).toBeTruthy();
  });

  it("maps Pluralsight commands to the backend invoke contract", async () => {
    const calls: Array<[string, Record<string, unknown> | undefined]> = [];
    const invoke = async <T>(command: string, args?: Record<string, unknown>): Promise<T> => {
      calls.push([command, args]);
      return undefined as T;
    };
    const commands = createPluralsightCommands(invoke);
    await commands.pluralsightLoginCredentials("session=synthetic");
    await commands.pluralsightLoginWebview();
    await commands.pluralsightSessionStatus();
    await commands.pluralsightLogout();
    await commands.pluralsightCourses();
    await commands.pluralsightDownloadStart("course-id");
    await commands.pluralsightDownloadCancel("download-id");
    await commands.pluralsightDownloadsRunning();
    expect(calls).toEqual([
      ["pluralsight_login_credentials", { text: "session=synthetic" }],
      ["pluralsight_login_webview", undefined],
      ["pluralsight_session_status", undefined],
      ["pluralsight_logout", undefined],
      ["pluralsight_courses", undefined],
      ["pluralsight_download_start", { course: "course-id" }],
      ["pluralsight_download_cancel", { id: "download-id" }],
      ["pluralsight_downloads_running", undefined],
    ]);
  });
});
