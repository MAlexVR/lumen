import { describe, expect, it } from "vitest";
import type { Group, Library } from "../types";
import { groupFolder, groupFolderName, localStatus, localStatusByFolder } from "./localStatus";

const g = (name: string, status: Group["status"], courses = 1, platform: Group["platform"] = "skool"): Group => ({
  id: platform === "hotmart" ? `hotmart/${name}` : name,
  platform,
  name,
  title: name,
  seller: null,
  lessonCount: 0,
  status,
  courses: Array.from({ length: courses }, (_, i) => ({ title: `C${i}`, index: i + 1, sections: [] })),
});

describe("groupFolderName", () => {
  it("mirrors the downloader: strips emoji and symbols, replaces illegal characters, trims", () => {
    expect(groupFolderName("🚀 Growth Lab ™")).toBe("Growth Lab");
    expect(groupFolderName("A/B: Test?")).toBe("A_B_ Test_");
    expect(groupFolderName("Ends with dot.")).toBe("Ends with dot");
    expect(groupFolderName("CON")).toBe("_CON");
    expect(groupFolderName("🔥")).toBe("_");
  });

  it("builds the library-relative folder path", () => {
    expect(groupFolder("🚀 Growth Lab")).toBe("Skool/Growth Lab");
  });
});

describe("localStatus", () => {
  const skool = [g("ACADEMIA AURORA", "complete"), g("Growth Lab", "partial"), g("Empty", "none", 0), g("Started", "none", 2)];
  const library: Library = {
    platforms: [
      { id: "skool", groups: skool },
      { id: "hotmart", groups: [g("Growth Lab", "complete", 1, "hotmart")] },
    ],
    groups: skool,
  };

  it("is none when the group is not on disk", () => {
    expect(localStatus({ name: "Unknown" }, library)).toBe("none");
  });

  it("uses the scanned status for a matching folder", () => {
    expect(localStatus({ name: "ACADEMIA AURORA" }, library)).toBe("complete");
    expect(localStatus({ name: "🚀 Growth Lab" }, library)).toBe("partial");
  });

  it("matches case-insensitively", () => {
    expect(localStatus({ name: "academia aurora" }, library)).toBe("complete");
  });

  it("treats a folder with courses but no marker as partial and an empty one as none", () => {
    expect(localStatus({ name: "Started" }, library)).toBe("partial");
    expect(localStatus({ name: "Empty" }, library)).toBe("none");
  });
});

describe("localStatusByFolder", () => {
  const library: Library = {
    platforms: [
      { id: "skool", groups: [g("Curso A - Vendedor", "partial")] },
      { id: "hotmart", groups: [g("Curso A - Vendedor", "complete", 1, "hotmart"), g("Curso B", "partial", 1, "hotmart")] },
    ],
    groups: [],
  };

  it("matches the Hotmart folder exactly within that platform", () => {
    expect(localStatusByFolder(library, "hotmart", "Curso A - Vendedor")).toBe("complete");
    expect(localStatusByFolder(library, "hotmart", "Curso B")).toBe("partial");
    expect(localStatusByFolder(library, "hotmart", "Curso C")).toBe("none");
  });

  it("does not match a folder of another platform", () => {
    expect(localStatusByFolder({ ...library, platforms: [library.platforms[0]] }, "hotmart", "Curso A - Vendedor")).toBe("none");
  });
});
