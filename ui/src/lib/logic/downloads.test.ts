import { describe, expect, it } from "vitest";
import type { ProgressEvent, ReportEvent } from "../types";
import { downloadFor, initialDownloads, overallPercent, reduceDownloads, type DownloadsState } from "./downloads";

const progress = (p: Partial<ProgressEvent> = {}): ProgressEvent => ({
  id: "dl-1",
  platform: "skool",
  group: "Growth Lab",
  course: "Course A",
  lesson: "Lesson 1",
  percent: 0,
  completed: 0,
  total: 10,
  ...p,
});

const report = (r: Partial<ReportEvent> = {}): ReportEvent => ({
  id: "dl-1",
  platform: "skool",
  group: "Growth Lab",
  cancelled: false,
  totalLessons: 10,
  completedLessons: 10,
  complete: true,
  issues: [],
  ...r,
});

const run = (...actions: Parameters<typeof reduceDownloads>[1][]): DownloadsState =>
  actions.reduce(reduceDownloads, initialDownloads);

describe("reduceDownloads", () => {
  it("tracks a started download by id and slug", () => {
    const s = run({ type: "started", id: "dl-1", platform: "skool", key: "growth" });
    expect(s["dl-1"]).toMatchObject({ id: "dl-1", platform: "skool", key: "growth", status: "running", completed: 0, total: 0 });
  });

  it("applies progress events", () => {
    const s = run({ type: "started", id: "dl-1", platform: "skool", key: "growth" }, { type: "progress", event: progress({ completed: 3, percent: 40, lesson: "L4" }) });
    expect(s["dl-1"]).toMatchObject({ name: "Growth Lab", course: "Course A", lesson: "L4", completed: 3, total: 10, percent: 40 });
  });

  it("ignores stale progress that would move completed backwards", () => {
    const s = run(
      { type: "started", id: "dl-1", platform: "skool", key: "growth" },
      { type: "progress", event: progress({ completed: 5, lesson: "L6" }) },
      { type: "progress", event: progress({ completed: 4, lesson: "L5" }) },
    );
    expect(s["dl-1"]).toMatchObject({ completed: 5, lesson: "L6" });
  });

  it("keeps several downloads independent", () => {
    const s = run(
      { type: "started", id: "dl-1", platform: "skool", key: "a" },
      { type: "started", id: "dl-2", platform: "skool", key: "b" },
      { type: "progress", event: progress({ id: "dl-2", group: "B", completed: 2 }) },
    );
    expect(s["dl-1"].completed).toBe(0);
    expect(s["dl-2"]).toMatchObject({ key: "b", name: "B", completed: 2 });
  });

  it("re-attaches running downloads without losing known progress", () => {
    const s = run(
      { type: "progress", event: progress({ id: "dl-7", completed: 2 }) },
      { type: "attached", running: [{ id: "dl-7", platform: "skool", group: "growth" }, { id: "dl-8", platform: "hotmart", group: "4711" }] },
    );
    expect(s["dl-7"]).toMatchObject({ platform: "skool", key: "growth", completed: 2, status: "running" });
    expect(s["dl-8"]).toMatchObject({ platform: "hotmart", key: "4711", status: "running" });
  });

  it("creates an entry for progress of an unknown id", () => {
    expect(run({ type: "progress", event: progress({ id: "dl-9", platform: "hotmart" }) })["dl-9"]).toMatchObject({
      platform: "hotmart",
      key: null,
      status: "running",
    });
  });

  it("finishes on report and ignores late progress", () => {
    const s = run(
      { type: "started", id: "dl-1", platform: "skool", key: "growth" },
      { type: "report", event: report({ completedLessons: 9 }) },
      { type: "progress", event: progress({ completed: 9 }) },
    );
    expect(s["dl-1"].status).toBe("finished");
    expect(s["dl-1"].report?.completedLessons).toBe(9);
    expect(s["dl-1"].completed).toBe(9);
  });

  it("dismisses a finished download", () => {
    const s = run({ type: "started", id: "dl-1", platform: "skool", key: "g" }, { type: "report", event: report() }, { type: "dismiss", id: "dl-1" });
    expect(s["dl-1"]).toBeUndefined();
  });

  it("does not mutate the previous state", () => {
    const a = run({ type: "started", id: "dl-1", platform: "skool", key: "g" });
    const b = reduceDownloads(a, { type: "progress", event: progress({ completed: 1 }) });
    expect(a["dl-1"].completed).toBe(0);
    expect(b).not.toBe(a);
  });
});

describe("selectors", () => {
  it("finds the download for a group by slug or display name, preferring running ones", () => {
    const s = run(
      { type: "started", id: "dl-1", platform: "skool", key: "growth" },
      { type: "report", event: report() },
      { type: "started", id: "dl-2", platform: "skool", key: "growth" },
      { type: "progress", event: progress({ id: "dl-3", group: "Other Group" }) },
    );
    expect(downloadFor(s, { platform: "skool", key: "growth", name: "Growth Lab" })?.id).toBe("dl-2");
    expect(downloadFor(s, { platform: "skool", key: "other", name: "Other Group" })?.id).toBe("dl-3");
    expect(downloadFor(s, { platform: "skool", key: "none", name: "None" })).toBeUndefined();
  });

  it("keeps concurrent downloads of different platforms apart", () => {
    const s = run(
      { type: "started", id: "dl-1", platform: "skool", key: "x" },
      { type: "started", id: "dl-2", platform: "hotmart", key: "x" },
      { type: "progress", event: progress({ id: "dl-2", platform: "hotmart", group: "Curso X", completed: 3 }) },
      { type: "report", event: { ...report({ id: "dl-1" }), platform: "skool" } },
    );
    expect(downloadFor(s, { platform: "hotmart", key: "x", name: "Curso X" })).toMatchObject({ id: "dl-2", completed: 3, status: "running" });
    expect(downloadFor(s, { platform: "skool", key: "x", name: "X" })).toMatchObject({ id: "dl-1", status: "finished" });
  });

  it("computes overall percent including the current lesson", () => {
    const s = run({ type: "started", id: "dl-1", platform: "skool", key: "g" }, { type: "progress", event: progress({ completed: 2, percent: 50, total: 10 }) });
    expect(overallPercent(s["dl-1"])).toBe(25);
    expect(overallPercent({ ...s["dl-1"], total: 0 })).toBe(0);
    expect(overallPercent({ ...s["dl-1"], completed: 10, percent: 100 })).toBe(100);
  });
});
