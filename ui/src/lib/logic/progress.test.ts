import { describe, expect, it } from "vitest";
import type { Course, Group, Lesson } from "../types";
import {
  continueTarget,
  courseStats,
  firstPlayable,
  flattenCourse,
  flattenGroup,
  nextUnwatched,
  sectionStats,
  stats, sectionKey, sectionNumber } from "./progress";

const lesson = (key: string, hasVideo = true): Lesson => ({
  key,
  title: key,
  index: 1,
  video: hasVideo ? `${key}.mp4` : null,
  description: null,
  hasVideo,
  resources: [],
});

const course = (index: number, sections: Lesson[][]): Course => ({
  title: `C${index}`,
  index,
  sections: sections.map((lessons, i) => ({ title: `S${i + 1}`, index: i + 1, lessons })),
});

const group: Group = {
  id: "G",
  platform: "skool",
  title: "G",
  seller: null,
  lessonCount: 5,
  name: "G",
  status: "complete",
  courses: [
    course(1, [[lesson("a", false), lesson("b")], [lesson("c")]]),
    course(2, [[lesson("d"), lesson("e")]]),
  ],
};

describe("stats", () => {
  it("is new with zero percent when nothing is watched", () => {
    expect(stats([lesson("a"), lesson("b")], new Set())).toEqual({
      total: 2,
      done: 0,
      pct: 0,
      state: "new",
      noVideo: 0,
    });
  });

  it("rounds the percentage and reports progress and missing videos", () => {
    const s = stats([lesson("a"), lesson("b"), lesson("c", false)], new Set(["a"]));
    expect(s).toEqual({ total: 3, done: 1, pct: 33, state: "progress", noVideo: 1 });
  });

  it("is done when every lesson is watched", () => {
    expect(stats([lesson("a")], new Set(["a"])).state).toBe("done");
  });

  it("treats an empty list as new, not done", () => {
    expect(stats([], new Set())).toMatchObject({ total: 0, pct: 0, state: "new" });
  });

  it("ignores watched keys that are not in the list", () => {
    expect(stats([lesson("a")], new Set(["zzz"])).done).toBe(0);
  });
});

describe("course and section stats", () => {
  it("aggregates all sections of a course", () => {
    expect(courseStats(group.courses[0], new Set(["b", "c"]))).toMatchObject({ total: 3, done: 2, pct: 67 });
  });

  it("counts one section only", () => {
    expect(sectionStats(group.courses[0].sections[0], new Set(["b"]))).toMatchObject({
      total: 2,
      done: 1,
      noVideo: 1,
    });
  });
});

describe("flatten", () => {
  it("keeps course and section context and a global order", () => {
    const flat = flattenGroup(group);
    expect(flat.map((f) => f.lesson.key)).toEqual(["a", "b", "c", "d", "e"]);
    expect(flat[2]).toMatchObject({ order: 2, course: { index: 1 }, section: { index: 2 } });
  });

  it("flattens a single course", () => {
    expect(flattenCourse(group.courses[1]).map((f) => f.lesson.key)).toEqual(["d", "e"]);
  });
});

describe("selection", () => {
  it("picks the first unwatched lesson that has a video", () => {
    expect(nextUnwatched(flattenGroup(group), new Set(["b"]))?.lesson.key).toBe("c");
  });

  it("falls back to an unwatched lesson without video", () => {
    const flat = flattenCourse(course(9, [[lesson("x", false), lesson("y")]]));
    expect(nextUnwatched(flat, new Set(["y"]))?.lesson.key).toBe("x");
  });

  it("falls back to the first lesson when everything is watched", () => {
    expect(nextUnwatched(flattenCourse(group.courses[1]), new Set(["d", "e"]))?.lesson.key).toBe("d");
  });

  it("returns null for an empty list", () => {
    expect(nextUnwatched([], new Set())).toBeNull();
    expect(firstPlayable([])).toBeNull();
  });

  it("first playable skips lessons without video", () => {
    expect(firstPlayable(flattenGroup(group))?.lesson.key).toBe("b");
  });
});

describe("continueTarget", () => {
  const flat = flattenGroup(group);

  it("resumes the last lesson with its saved position", () => {
    const t = continueTarget(flat, {
      watched: [],
      lastLesson: "c",
      positions: [{ lessonKey: "c", seconds: 95 }],
    });
    expect(t).toMatchObject({ kind: "resume", seconds: 95, item: { lesson: { key: "c" } } });
  });

  it("starts at the first playable lesson without history", () => {
    const t = continueTarget(flat, { watched: [], lastLesson: null, positions: [] });
    expect(t).toMatchObject({ kind: "start", seconds: 0, item: { lesson: { key: "b" } } });
  });

  it("ignores a last lesson that no longer exists", () => {
    const t = continueTarget(flat, { watched: [], lastLesson: "gone", positions: [] });
    expect(t?.kind).toBe("start");
  });

  it("is null for an empty group", () => {
    expect(continueTarget([], { watched: [], lastLesson: null, positions: [] })).toBeNull();
  });
});

describe("sectionKey", () => {
  it("tells apart modules that share an index, as unnumbered folders do", () => {
    const months = ["2026-06", "2026-07", "2026-08"].map((title) => ({ title, index: 0, lessons: [] }));
    expect(new Set(months.map(sectionKey)).size).toBe(3);
  });

  it("is stable for the same module", () => {
    expect(sectionKey({ title: "Intro", index: 2, lessons: [] })).toBe(sectionKey({ title: "Intro", index: 2, lessons: [] }));
  });
});

describe("sectionNumber", () => {
  it("keeps the number a module carries", () => {
    expect(sectionNumber({ title: "Intro", index: 3, lessons: [] }, 0)).toBe(3);
  });

  it("falls back to the position for unnumbered modules such as month folders", () => {
    expect(sectionNumber({ title: "2026-08", index: 0, lessons: [] }, 2)).toBe(3);
  });
});
