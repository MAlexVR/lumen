import { describe, expect, it } from "vitest";
import type { Course, Group } from "../types";
import { courseFolder, deleteTarget, formatBytes, initialDelete, reduceDelete, toDeleteError, type DeletePreview } from "./deletion";

const lesson = (video: string | null, description: string | null = null) => ({
  key: "k",
  title: "t",
  index: 1,
  video,
  description,
  hasVideo: !!video,
  resources: [],
});
const course = (index: number, title: string, lessons: ReturnType<typeof lesson>[]): Course => ({
  title,
  index,
  sections: [{ title: "S", index: 1, lessons }],
});
const group = (g: Partial<Group> & Pick<Group, "id" | "platform" | "name" | "courses">): Group => ({
  title: g.name,
  seller: null,
  status: "complete",
  lessonCount: 0,
  ...g,
});

describe("courseFolder", () => {
  const g = group({ id: "ACADEMIA AURORA", platform: "skool", name: "ACADEMIA AURORA", courses: [] });

  it("reads the folder from the lesson paths", () => {
    const c = course(1, "COMIENZA TU RECORRIDO", [lesson("Skool/ACADEMIA AURORA/01 - COMIENZA TU RECORRIDO/01 - S/01 - L/v.mp4")]);
    expect(courseFolder(g, c)).toBe("01 - COMIENZA TU RECORRIDO");
  });

  it("uses a description path when the lesson has no video", () => {
    const c = course(3, "X", [lesson(null, "Skool/ACADEMIA AURORA/03 - X_ Y/01 - S/02 - L/description.html")]);
    expect(courseFolder(g, c)).toBe("03 - X_ Y");
  });

  it("falls back to the numbered title when no path is available", () => {
    expect(courseFolder(g, course(7, "Modo Cabaña", [lesson(null)]))).toBe("07 - Modo Cabaña");
    expect(courseFolder(g, course(112, "Bonus", []))).toBe("112 - Bonus");
  });

  it("ignores paths of another group", () => {
    const c = course(2, "B", [lesson("Skool/OTHER/09 - Z/01 - S/01 - L/v.mp4")]);
    expect(courseFolder(g, c)).toBe("02 - B");
  });
});

describe("deleteTarget", () => {
  const skool = group({ id: "ACADEMIA AURORA", platform: "skool", name: "ACADEMIA AURORA", courses: [course(1, "A", [lesson("Skool/ACADEMIA AURORA/01 - A/01 - S/01 - L/v.mp4")])] });
  const hot = group({ id: "hotmart/Curso - Vendedor", platform: "hotmart", name: "Curso - Vendedor", courses: [course(1, "Curso", [])] });

  it("targets a whole Skool group by id", () => {
    expect(deleteTarget(skool)).toEqual({ platform: "skool", group: "ACADEMIA AURORA" });
  });

  it("targets a Skool course by its folder name", () => {
    expect(deleteTarget(skool, skool.courses[0])).toEqual({ platform: "skool", group: "ACADEMIA AURORA", course: "01 - A" });
  });

  it("always targets the whole Hotmart purchase", () => {
    expect(deleteTarget(hot, hot.courses[0])).toEqual({ platform: "hotmart", group: "hotmart/Curso - Vendedor" });
  });

  it("always targets the whole YouTube group, never a course", () => {
    const yt = group({ id: "youtube/Videos sueltos", platform: "youtube", name: "Videos sueltos", courses: [course(1, "Videos sueltos", [])] });
    expect(deleteTarget(yt, yt.courses[0])).toEqual({ platform: "youtube", group: "youtube/Videos sueltos" });
  });

  it("never carries a path", () => {
    expect(JSON.stringify(deleteTarget(skool, skool.courses[0]))).not.toContain("/01 -");
  });
});

const preview: DeletePreview = {
  name: "A",
  platform: "skool",
  kind: "course",
  lessons: 5,
  lessonsWithVideo: 4,
  bytes: 1_500_000_000,
  bytesApprox: false,
  path: "Skool/G/01 - A",
  runningDownload: false,
};

describe("reduceDelete", () => {
  const target = { platform: "skool" as const, group: "G", course: "01 - A" };

  it("walks idle → previewing → confirming → deleting → done", () => {
    let s = reduceDelete(initialDelete, { type: "open", target, title: "A" });
    expect(s).toMatchObject({ step: "previewing", title: "A" });
    s = reduceDelete(s, { type: "previewed", preview });
    expect(s).toMatchObject({ step: "confirming", preview });
    s = reduceDelete(s, { type: "confirm", permanent: false });
    expect(s).toMatchObject({ step: "deleting", permanent: false });
    s = reduceDelete(s, { type: "deleted", result: { deleted: true, movedToTrash: true, bytesFreed: 10, bytesApprox: false, groupRemoved: false } });
    expect(s).toMatchObject({ step: "done", result: { movedToTrash: true } });
  });

  it("does not confirm while a download of the platform is running", () => {
    const s = reduceDelete(
      { step: "confirming", target, title: "A", preview: { ...preview, runningDownload: true } },
      { type: "confirm", permanent: false },
    );
    expect(s.step).toBe("confirming");
  });

  it("keeps the preview on failure so the dialog can explain and offer the next step", () => {
    const s = reduceDelete(
      { step: "deleting", target, title: "A", preview, permanent: false },
      { type: "failed", error: { code: "trash_failed", message: "no trash" } },
    );
    expect(s).toMatchObject({ step: "error", preview, error: { code: "trash_failed" } });
    expect(reduceDelete(s, { type: "confirm", permanent: true })).toMatchObject({ step: "deleting", permanent: true });
  });

  it("reports a failed preview without a preview", () => {
    const s = reduceDelete(
      { step: "previewing", target, title: "A" },
      { type: "failed", error: { code: "not_found", message: "gone" } },
    );
    expect(s).toMatchObject({ step: "error", preview: null, error: { code: "not_found" } });
  });

  it("closes from any step", () => {
    expect(reduceDelete({ step: "previewing", target, title: "A" }, { type: "close" })).toEqual(initialDelete);
  });
});

describe("toDeleteError", () => {
  it.each(["invalid_target", "not_found", "download_running", "trash_failed", "internal"])("keeps %s", (code) => {
    expect(toDeleteError({ code, message: "m" }).code).toBe(code);
  });

  it("maps anything else to internal", () => {
    expect(toDeleteError("boom")).toEqual({ code: "internal", message: "boom" });
    expect(toDeleteError({ code: "weird", message: "x" }).code).toBe("internal");
  });
});

describe("formatBytes", () => {
  it("uses binary-friendly decimal units for the locale", () => {
    expect(formatBytes(0, false, "es")).toBe("0 B");
    expect(formatBytes(950, false, "es")).toBe("950 B");
    expect(formatBytes(1_500_000, false, "es")).toBe("1,5 MB");
    expect(formatBytes(1_500_000_000, false, "en")).toBe("1.5 GB");
    expect(formatBytes(2_000_000_000_000, false, "en")).toBe("2 TB");
  });

  it("prefixes approximate sizes", () => {
    expect(formatBytes(1_500_000_000, true, "es")).toBe("~1,5 GB");
  });
});
