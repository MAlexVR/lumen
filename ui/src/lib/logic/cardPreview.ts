// Poster card previews: which lesson a card shows and the hover-play state.
import type { Course, GroupProgress, Lesson } from "../types";
import { previewSrc } from "./preview";
import { positionOf } from "./progress";

/** The lesson a course card previews: the last opened one in this course
 * (with its resume position) or the first lesson with a video. */
export function representativeLesson(course: Course, progress: GroupProgress): { lesson: Lesson; seconds: number } | null {
  const lessons = course.sections.flatMap((s) => s.lessons);
  const last = lessons.find((l) => l.key === progress.lastLesson && l.hasVideo && l.video);
  if (last) return { lesson: last, seconds: positionOf(progress, last.key) };
  const first = lessons.find((l) => l.hasVideo && l.video);
  return first ? { lesson: first, seconds: 0 } : null;
}

export type HoverState = { mode: "still" } | { mode: "playing"; restart?: boolean };
export type HoverAction =
  | { type: "enter"; canHover: boolean; reducedMotion: boolean }
  | { type: "leave" }
  | { type: "tick"; elapsed: number; window: number };

export const initialHover: HoverState = { mode: "still" };

export function reduceHover(s: HoverState, a: HoverAction): HoverState {
  switch (a.type) {
    case "enter":
      return a.canHover && !a.reducedMotion ? { mode: "playing" } : s;
    case "leave":
      return initialHover;
    case "tick":
      if (s.mode !== "playing") return s;
      return a.elapsed >= a.window ? { mode: "playing", restart: true } : { mode: "playing" };
  }
}

/** What a poster card previews, or null to keep the generated poster. */
export function cardPreviewFor(
  course: Course,
  progress: GroupProgress,
  mediaBase: string,
): { src: string; path: string; resumeAt: number } | null {
  const rep = representativeLesson(course, progress);
  const src = rep ? previewSrc(mediaBase, rep.lesson) : null;
  return rep && src && rep.lesson.video ? { src, path: rep.lesson.video, resumeAt: rep.seconds } : null;
}
