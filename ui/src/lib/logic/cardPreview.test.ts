import { describe, expect, it } from "vitest";
import type { Course, GroupProgress, Lesson } from "../types";
import { cardPreviewFor, initialHover, reduceHover, representativeLesson } from "./cardPreview";

const lesson = (key: string, video = true): Lesson => ({
  key,
  title: key,
  index: 1,
  video: video ? `${key}.mp4` : null,
  description: null,
  hasVideo: video,
  resources: [],
});
const course: Course = { title: "C", index: 1, sections: [{ title: "S", index: 1, lessons: [lesson("a", false), lesson("b"), lesson("c")] }] };
const p = (x: Partial<GroupProgress> = {}): GroupProgress => ({ watched: [], lastLesson: null, positions: [], ...x });

describe("representativeLesson", () => {
  it("prefers the last opened lesson of the course with its resume position", () => {
    expect(representativeLesson(course, p({ lastLesson: "c", positions: [{ lessonKey: "c", seconds: 80 }] }))).toMatchObject({
      lesson: { key: "c" },
      seconds: 80,
    });
  });

  it("falls back to the first lesson with a video when the last one is elsewhere or has none", () => {
    expect(representativeLesson(course, p({ lastLesson: "zzz" }))).toMatchObject({ lesson: { key: "b" }, seconds: 0 });
    expect(representativeLesson(course, p({ lastLesson: "a" }))).toMatchObject({ lesson: { key: "b" }, seconds: 0 });
  });

  it("is null for a course without videos", () => {
    const silent: Course = { ...course, sections: [{ title: "S", index: 1, lessons: [lesson("x", false)] }] };
    expect(representativeLesson(silent, p())).toBeNull();
  });
});

describe("reduceHover", () => {
  const allowed = { canHover: true, reducedMotion: false };

  it("plays on hover or focus only where hover is real and motion is allowed", () => {
    expect(reduceHover(initialHover, { type: "enter", ...allowed })).toEqual({ mode: "playing" });
    expect(reduceHover(initialHover, { type: "enter", canHover: false, reducedMotion: false })).toEqual(initialHover);
    expect(reduceHover(initialHover, { type: "enter", canHover: true, reducedMotion: true })).toEqual(initialHover);
  });

  it("returns to the still frame on leave", () => {
    expect(reduceHover({ mode: "playing" }, { type: "leave" })).toEqual(initialHover);
  });

  it("loops back after the preview window while still hovered", () => {
    expect(reduceHover({ mode: "playing" }, { type: "tick", elapsed: 5.2, window: 5 })).toEqual({ mode: "playing", restart: true });
    expect(reduceHover({ mode: "playing" }, { type: "tick", elapsed: 2, window: 5 })).toEqual({ mode: "playing" });
    expect(reduceHover(initialHover, { type: "tick", elapsed: 9, window: 5 })).toEqual(initialHover);
  });
});

describe("cardPreviewFor", () => {
  it("builds the media URL and resume position of the representative lesson", () => {
    expect(cardPreviewFor(course, p({ lastLesson: "c", positions: [{ lessonKey: "c", seconds: 30 }] }), "http://127.0.0.1:9/m/t")).toEqual({
      src: "http://127.0.0.1:9/m/t/c.mp4",
      path: "c.mp4",
      resumeAt: 30,
    });
  });

  it("falls back to the poster without a media server or without videos", () => {
    expect(cardPreviewFor(course, p(), "")).toBeNull();
    const silent: Course = { ...course, sections: [{ title: "S", index: 1, lessons: [lesson("x", false)] }] };
    expect(cardPreviewFor(silent, p(), "http://127.0.0.1:9/m/t")).toBeNull();
  });
});

describe("representative change", () => {
  it("moves to the new lesson when the continue lesson changes or the first video appears", () => {
    const base = "http://127.0.0.1:9/m/t";
    expect(cardPreviewFor(course, p({ lastLesson: "b" }), base)?.path).toBe("b.mp4");
    expect(cardPreviewFor(course, p({ lastLesson: "c" }), base)?.path).toBe("c.mp4");
    const none: Course = { ...course, sections: [{ title: "S", index: 1, lessons: [lesson("a", false)] }] };
    expect(cardPreviewFor(none, p(), base)).toBeNull();
    const later: Course = { ...none, sections: [{ title: "S", index: 1, lessons: [lesson("a", true)] }] };
    expect(cardPreviewFor(later, p(), base)?.path).toBe("a.mp4");
  });
});
