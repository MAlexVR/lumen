import { describe, expect, it } from "vitest";
import type { Lesson } from "../types";
import { exerciseTitle, lessonVideos, stepVideo } from "./lessonVideos";

const lesson = (over: Partial<Lesson> = {}): Lesson => ({
  key: "k",
  title: "2026-06-01 - Mobility",
  index: 0,
  video: "Harbiz/T/2026-06/c/01. Bodyweight squat.mp4",
  description: "Harbiz/T/2026-06/c/Description.html",
  videoNote: "Harbiz/T/2026-06/c/01. Bodyweight squat.html",
  hasVideo: true,
  resources: [
    { name: "03. Walking  lunge.mp4", path: "Harbiz/T/2026-06/c/03. Walking  lunge.mp4", url: null, kind: "video", description: "Harbiz/T/2026-06/c/03. Walking  lunge.html" },
    { name: "guide.pdf", path: "Harbiz/T/2026-06/c/guide.pdf", url: null, kind: "file" },
    { name: "04. Plank hold.mp4", path: "Harbiz/T/2026-06/c/04. Plank hold.mp4", url: null, kind: "video" },
  ],
  ...over,
});

describe("exerciseTitle", () => {
  it("drops the number, the extension and doubled spaces", () => {
    expect(exerciseTitle("03. Walking  lunge.mp4")).toBe("Walking lunge");
    expect(exerciseTitle("Harbiz/T/c/10. Side lunge reach.MKV")).toBe("Side lunge reach");
  });

  it("keeps a plain name and strips a YouTube id tag", () => {
    expect(exerciseTitle("Intro.mp4")).toBe("Intro");
    expect(exerciseTitle("2. Lesson [dQw4w9WgXcQ].mp4")).toBe("Lesson");
  });
});

describe("lessonVideos", () => {
  it("lists the main video and the further ones in order, each with its own note", () => {
    expect(lessonVideos(lesson())).toEqual([
      { path: "Harbiz/T/2026-06/c/01. Bodyweight squat.mp4", title: "Bodyweight squat", note: "Harbiz/T/2026-06/c/01. Bodyweight squat.html" },
      { path: "Harbiz/T/2026-06/c/03. Walking  lunge.mp4", title: "Walking lunge", note: "Harbiz/T/2026-06/c/03. Walking  lunge.html" },
      { path: "Harbiz/T/2026-06/c/04. Plank hold.mp4", title: "Plank hold", note: null },
    ]);
  });

  it("is empty for a lesson without a video", () => {
    expect(lessonVideos(lesson({ video: null, hasVideo: false, resources: [] }))).toEqual([]);
  });

  it("treats a lesson from an older scan (no note fields) as plain videos", () => {
    const old = lesson({ videoNote: undefined, resources: [] });
    expect(lessonVideos(old)).toEqual([{ path: "Harbiz/T/2026-06/c/01. Bodyweight squat.mp4", title: "Bodyweight squat", note: null }]);
  });
});

describe("stepVideo", () => {
  it("moves to the next video inside the lesson", () => {
    expect(stepVideo(5, 1, 1)).toEqual({ kind: "video", index: 2 });
    expect(stepVideo(5, 3, -1)).toEqual({ kind: "video", index: 2 });
  });

  it("leaves the lesson after the last video going forward and before the first going back", () => {
    expect(stepVideo(5, 4, 1)).toEqual({ kind: "lesson" });
    expect(stepVideo(5, 0, -1)).toEqual({ kind: "lesson" });
  });

  it("a lesson with one video always moves to another lesson", () => {
    expect(stepVideo(1, 0, 1)).toEqual({ kind: "lesson" });
    expect(stepVideo(1, 0, -1)).toEqual({ kind: "lesson" });
    expect(stepVideo(0, 0, 1)).toEqual({ kind: "lesson" });
  });
});
