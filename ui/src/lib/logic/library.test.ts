import { describe, expect, it } from "vitest";
import type { Course, Group, GroupProgress, Lesson, Library } from "../types";
import {
  allGroups,
  continueAcross,
  groupById,
  libraryEntries,
  opensStraightIntoCourse,
  platformsPresent,
  shelves,
} from "./library";

const lesson = (key: string, hasVideo = true): Lesson => ({
  key,
  title: key,
  index: 1,
  video: hasVideo ? `${key}.mp4` : null,
  description: null,
  hasVideo,
  resources: [],
});
const course = (index: number, title: string, keys: string[]): Course => ({
  title,
  index,
  sections: [{ title: "M1", index: 1, lessons: keys.map((k) => lesson(k)) }],
});
const group = (g: Partial<Group> & Pick<Group, "id" | "platform" | "courses">): Group => ({
  name: g.id,
  title: g.id,
  seller: null,
  status: "complete",
  lessonCount: 0,
  ...g,
});

const skool = group({
  id: "ACADEMIA AURORA",
  platform: "skool",
  title: "ACADEMIA AURORA",
  courses: [course(1, "Inicia", ["s1", "s2"]), course(2, "Modo Cabaña", ["s3"])],
});
const hot1 = group({
  id: "hotmart/Fotografía - Estudio Norte",
  platform: "hotmart",
  name: "Fotografía - Estudio Norte",
  title: "Fotografía",
  seller: "Estudio Norte",
  courses: [course(1, "Fotografía", ["h1", "h2"])],
});
const hot2 = group({
  id: "hotmart/Finanzas",
  platform: "hotmart",
  title: "Finanzas",
  courses: [course(1, "Finanzas", ["f1"])],
});
const ps1 = group({
  id: "pluralsight/JavaScript Fundamentals-0123456789abcdef",
  platform: "pluralsight",
  title: "JavaScript Fundamentals",
  courses: [course(1, "JavaScript Fundamentals", ["ps1"])],
});
const gl1 = group({
  id: "goldenlife/Creator Basics",
  platform: "goldenlife",
  title: "Creator Basics",
  courses: [course(1, "Creator Basics", ["gl1"])],
});
const hb1 = group({
  id: "harbiz/Harbiz Training",
  platform: "harbiz",
  title: "Harbiz Training",
  courses: [course(1, "Harbiz Training", ["hb1"])],
});
const library: Library = {
  platforms: [
    { id: "skool", groups: [skool] },
    { id: "hotmart", groups: [hot1, hot2] },
    { id: "pluralsight", groups: [ps1] },
    { id: "goldenlife", groups: [gl1] },
    { id: "harbiz", groups: [hb1] },
  ],
  groups: [skool],
};
const none: GroupProgress = { watched: [], lastLesson: null, positions: [] };

describe("library selectors", () => {
  it("lists groups of every platform", () => {
    expect(allGroups(library).map((g) => g.id)).toEqual(["ACADEMIA AURORA", "hotmart/Fotografía - Estudio Norte", "hotmart/Finanzas", "pluralsight/JavaScript Fundamentals-0123456789abcdef", "goldenlife/Creator Basics", "harbiz/Harbiz Training"]);
  });

  it("falls back to the deprecated groups list without platforms", () => {
    expect(allGroups({ groups: [skool] } as unknown as Library).map((g) => g.id)).toEqual(["ACADEMIA AURORA"]);
  });

  it("finds a group by id, not by name", () => {
    expect(groupById(library, "hotmart/Finanzas")?.title).toBe("Finanzas");
    expect(groupById(library, "Fotografía - Estudio Norte")).toBeUndefined();
  });

  it("reports only platforms that have groups", () => {
    expect(platformsPresent(library)).toEqual(["skool", "hotmart", "pluralsight", "goldenlife", "harbiz"]);
    expect(platformsPresent({ platforms: [{ id: "skool", groups: [skool] }, { id: "hotmart", groups: [] }], groups: [] })).toEqual(["skool"]);
  });

  it("opens single-course Hotmart and Pluralsight groups straight into the course", () => {
    expect(opensStraightIntoCourse(hot1)).toBe(true);
    expect(opensStraightIntoCourse(ps1)).toBe(true);
    expect(opensStraightIntoCourse(skool)).toBe(false);
  });
});

describe("libraryEntries", () => {
  it("returns every group with its courses for all platforms", () => {
    const e = libraryEntries(library, null, "");
    expect(e.map((x) => [x.group.id, x.courses.length])).toEqual([
      ["ACADEMIA AURORA", 2],
      ["hotmart/Fotografía - Estudio Norte", 1],
      ["hotmart/Finanzas", 1],
      ["pluralsight/JavaScript Fundamentals-0123456789abcdef", 1],
      ["goldenlife/Creator Basics", 1],
      ["harbiz/Harbiz Training", 1],
    ]);
  });

  it("filters by platform", () => {
    expect(libraryEntries(library, "hotmart", "").map((x) => x.group.id)).toEqual(["hotmart/Fotografía - Estudio Norte", "hotmart/Finanzas"]);
  });

  it("searches course titles across platforms and drops empty groups", () => {
    const e = libraryEntries(library, null, "fotografia");
    expect(e.map((x) => x.group.id)).toEqual(["hotmart/Fotografía - Estudio Norte"]);
    expect(libraryEntries(library, null, "cabaña")[0].courses.map((c) => c.title)).toEqual(["Modo Cabaña"]);
  });

  it("matches the displayed title of an emoji-prefixed course", () => {
    const lib: Library = {
      platforms: [{ id: "skool", groups: [group({ id: "G", platform: "skool", courses: [course(1, "👋 Bienvenida 🚀", ["x"])] })] }],
      groups: [],
    };
    expect(libraryEntries(lib, null, "bienvenida").length).toBe(1);
    expect(libraryEntries(lib, null, "👋").length).toBe(1);
  });

  it("matches a Hotmart seller", () => {
    expect(libraryEntries(library, null, "estudio norte").map((x) => x.group.id)).toEqual(["hotmart/Fotografía - Estudio Norte"]);
  });
});

describe("shelves", () => {
  it("has one shelf per Skool group and one shelf for all Hotmart courses", () => {
    const s = shelves(library);
    expect(s.map((x) => [x.key, x.platform, x.items.length])).toEqual([
      ["ACADEMIA AURORA", "skool", 2],
      ["hotmart", "hotmart", 2],
      ["pluralsight", "pluralsight", 1],
      ["goldenlife", "goldenlife", 1],
      ["harbiz", "harbiz", 1],
    ]);
    expect(s[1].items.map((i) => i.group.id)).toEqual(["hotmart/Fotografía - Estudio Norte", "hotmart/Finanzas"]);
    expect(s[2].items[0].group.id).toBe("pluralsight/JavaScript Fundamentals-0123456789abcdef");
  });
});

describe("continueAcross", () => {
  const progress = (p: Record<string, Partial<GroupProgress>>) => (id: string): GroupProgress => ({ ...none, ...p[id] });

  it("continues the most recently used group (by id) across platforms", () => {
    const c = continueAcross(library, progress({ "ACADEMIA AURORA": { lastLesson: "s2" }, "hotmart/Finanzas": { lastLesson: "f1", positions: [{ lessonKey: "f1", seconds: 70 }] } }), "hotmart/Finanzas");
    expect(c?.group.id).toBe("hotmart/Finanzas");
    expect(c?.target).toMatchObject({ kind: "resume", seconds: 70, item: { lesson: { key: "f1" } } });
  });

  it("falls back to the first group with history when the recent one has none", () => {
    const c = continueAcross(library, progress({ "hotmart/Fotografía - Estudio Norte": { lastLesson: "h2" } }), "hotmart/Finanzas");
    expect(c?.group.id).toBe("hotmart/Fotografía - Estudio Norte");
    expect(c?.target.kind).toBe("resume");
  });

  it("starts the first group without any history", () => {
    const c = continueAcross(library, progress({}), null);
    expect(c).toMatchObject({ group: { id: "ACADEMIA AURORA" }, target: { kind: "start", item: { lesson: { key: "s1" } } } });
  });

  it("does not point at a deleted group or course", () => {
    // The recent group was deleted: continue falls back to the remaining groups.
    const afterGroup: Library = { platforms: [{ id: "skool", groups: [skool] }, { id: "hotmart", groups: [hot1] }], groups: [skool] };
    expect(continueAcross(afterGroup, progress({ "hotmart/Fotografía - Estudio Norte": { lastLesson: "h2" } }), "hotmart/Finanzas")?.group.id).toBe(
      "hotmart/Fotografía - Estudio Norte",
    );
    // The last lesson's course was deleted from a Skool group: start again.
    const trimmed = { ...skool, courses: [skool.courses[1]] };
    const afterCourse: Library = { platforms: [{ id: "skool", groups: [trimmed] }], groups: [trimmed] };
    const c = continueAcross(afterCourse, progress({ "ACADEMIA AURORA": { lastLesson: "s1" } }), "ACADEMIA AURORA");
    expect(c).toMatchObject({ target: { kind: "start", item: { lesson: { key: "s3" } } } });
  });

  it("is null for an empty library", () => {
    expect(continueAcross({ platforms: [], groups: [] }, progress({}), null)).toBeNull();
  });
});

describe("YouTube in the library", () => {
  const playlist = group({
    id: "youtube/Curso de guitarra",
    platform: "youtube",
    name: "Curso de guitarra",
    title: "Curso de guitarra",
    courses: [course(1, "Curso de guitarra", ["y1", "y2"])],
  });
  const singles = group({
    id: "youtube/Videos sueltos",
    platform: "youtube",
    name: "Videos sueltos",
    title: "Videos sueltos",
    courses: [course(1, "Videos sueltos", ["y3"])],
  });
  const lib: Library = {
    platforms: [
      { id: "youtube", groups: [playlist, singles] },
      { id: "skool", groups: [skool] },
      { id: "hotmart", groups: [hot1] },
    ],
    groups: [skool],
  };

  it("lists YouTube after Skool and Hotmart", () => {
    expect(allGroups(lib).map((g) => g.platform)).toEqual(["skool", "hotmart", "youtube", "youtube"]);
    expect(platformsPresent(lib)).toEqual(["skool", "hotmart", "youtube"]);
  });

  it("opens playlists and loose videos straight into their list", () => {
    expect(opensStraightIntoCourse(playlist)).toBe(true);
    expect(opensStraightIntoCourse(singles)).toBe(true);
  });

  it("has one shelf for all YouTube groups", () => {
    const s = shelves(lib);
    expect(s.map((x) => x.key)).toEqual(["ACADEMIA AURORA", "hotmart", "youtube"]);
    expect(s[2].items.map((i) => i.group.id)).toEqual(["youtube/Curso de guitarra", "youtube/Videos sueltos"]);
  });

  it("filters and searches YouTube groups", () => {
    expect(libraryEntries(lib, "youtube", "").map((e) => e.group.id)).toEqual(["youtube/Curso de guitarra", "youtube/Videos sueltos"]);
    expect(libraryEntries(lib, null, "guitarra").map((e) => e.group.id)).toEqual(["youtube/Curso de guitarra"]);
  });

  it("continues a YouTube lesson across platforms", () => {
    const none = { watched: [], lastLesson: null, positions: [] };
    const c = continueAcross(lib, (id) => (id === "youtube/Curso de guitarra" ? { ...none, lastLesson: "y2" } : none), "youtube/Curso de guitarra");
    expect(c).toMatchObject({ group: { id: "youtube/Curso de guitarra" }, target: { kind: "resume", item: { lesson: { key: "y2" } } } });
  });
});
