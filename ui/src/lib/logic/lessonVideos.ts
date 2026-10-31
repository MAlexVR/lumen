// A lesson can hold several videos (a workout: one per exercise). The player
// walks through them in order and shows the note of the one that is playing.
import type { Lesson } from "../types";

export interface LessonVideo {
  path: string;
  /** What the video is, from its file name: `03. Walking  lunge.mp4` -> `Walking lunge`. */
  title: string;
  /** Root-relative `.html` with only what belongs to this video, if it has one. */
  note: string | null;
}

export function exerciseTitle(fileName: string): string {
  const name = fileName.split("/").pop() ?? fileName;
  return name
    .replace(/\.[A-Za-z0-9]+$/, "")
    .replace(/^\d+\.\s+/, "")
    .replace(/\s*\[[\w-]{11}\]$/, "")
    .replace(/\s{2,}/g, " ")
    .trim();
}

export function lessonVideos(lesson: Lesson): LessonVideo[] {
  if (!lesson.video) return [];
  const further = lesson.resources.filter((r) => r.kind === "video");
  return [
    { path: lesson.video, title: exerciseTitle(lesson.video), note: lesson.videoNote ?? null },
    ...further.map((r) => ({ path: r.path, title: exerciseTitle(r.name), note: r.description ?? null })),
  ];
}

export type VideoStep = { kind: "video"; index: number } | { kind: "lesson" };

/** Next/previous inside the lesson first; `lesson` means move on to the neighbouring lesson. */
export function stepVideo(count: number, index: number, dir: 1 | -1): VideoStep {
  const target = index + dir;
  return target >= 0 && target < count ? { kind: "video", index: target } : { kind: "lesson" };
}
