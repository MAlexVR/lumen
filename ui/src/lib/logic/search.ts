// Client-side title search (FTS5 in SQLite comes later).
import type { Course } from "../types";

export const normalize = (s: string): string =>
  s
    .normalize("NFD")
    .replace(/\p{M}+/gu, "")
    .toLowerCase()
    .replace(/\s+/g, " ")
    .trim();

/** Courses whose title or any lesson title contains every query word. */
export function filterCourses(courses: readonly Course[], query: string): Course[] {
  const words = normalize(query).split(" ").filter(Boolean);
  if (!words.length) return [...courses];
  return courses.filter((c) => {
    const haystack = normalize(
      [c.title, ...c.sections.flatMap((s) => [s.title, ...s.lessons.map((l) => l.title)])].join(" "),
    );
    return words.every((w) => haystack.includes(w));
  });
}
