// Pure progress rules shared by Home, Course and Player.
import type { Course, Group, GroupProgress, Lesson, Section } from "../types";

export type ProgressState = "new" | "progress" | "done";

export interface Stats {
  total: number;
  done: number;
  pct: number;
  state: ProgressState;
  noVideo: number;
}

export interface FlatLesson {
  /** Position inside the flattened list it came from. */
  order: number;
  course: Course;
  section: Section;
  lesson: Lesson;
}

export interface ContinueTarget {
  kind: "resume" | "start";
  item: FlatLesson;
  seconds: number;
}

export function stats(lessons: readonly Lesson[], watched: ReadonlySet<string>): Stats {
  const total = lessons.length;
  const done = lessons.filter((l) => watched.has(l.key)).length;
  const pct = total ? Math.round((done * 100) / total) : 0;
  const state: ProgressState = done === 0 ? "new" : done === total ? "done" : "progress";
  return { total, done, pct, state, noVideo: lessons.filter((l) => !l.hasVideo).length };
}

/** Identity of a module inside a course. The index alone is not unique: unnumbered folders all read 0. */
export const sectionKey = (section: Section): string => `${section.index}:${section.title}`;

/** Number shown for a module: its own, or its position when the folder is unnumbered. */
export const sectionNumber = (section: Section, position: number): number => (section.index > 0 ? section.index : position + 1);

export const sectionStats = (section: Section, watched: ReadonlySet<string>): Stats =>
  stats(section.lessons, watched);

export const courseStats = (course: Course, watched: ReadonlySet<string>): Stats =>
  stats(
    course.sections.flatMap((s) => s.lessons),
    watched,
  );

export const groupStats = (group: Group, watched: ReadonlySet<string>): Stats =>
  stats(
    group.courses.flatMap((c) => c.sections.flatMap((s) => s.lessons)),
    watched,
  );

function flatten(courses: readonly Course[]): FlatLesson[] {
  const out: FlatLesson[] = [];
  for (const course of courses)
    for (const section of course.sections)
      for (const lesson of section.lessons) out.push({ order: out.length, course, section, lesson });
  return out;
}

export const flattenGroup = (group: Group): FlatLesson[] => flatten(group.courses);
export const flattenCourse = (course: Course): FlatLesson[] => flatten([course]);

/** First unwatched lesson, preferring ones with a video; first lesson when all are watched. */
export function nextUnwatched(flat: readonly FlatLesson[], watched: ReadonlySet<string>): FlatLesson | null {
  const unwatched = flat.filter((f) => !watched.has(f.lesson.key));
  return unwatched.find((f) => f.lesson.hasVideo) ?? unwatched[0] ?? flat[0] ?? null;
}

export const firstPlayable = (flat: readonly FlatLesson[]): FlatLesson | null =>
  flat.find((f) => f.lesson.hasVideo) ?? flat[0] ?? null;

export const positionOf = (progress: GroupProgress, key: string): number =>
  progress.positions.find((p) => p.lessonKey === key)?.seconds ?? 0;

/** "Continue where you left off": the last opened lesson, or where to start. */
export function continueTarget(flat: readonly FlatLesson[], progress: GroupProgress): ContinueTarget | null {
  const last = progress.lastLesson ? flat.find((f) => f.lesson.key === progress.lastLesson) : undefined;
  if (last) return { kind: "resume", item: last, seconds: positionOf(progress, last.lesson.key) };
  const first = firstPlayable(flat);
  return first ? { kind: "start", item: first, seconds: 0 } : null;
}
