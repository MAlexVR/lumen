import type { PluralsightCourse } from "../types";

export interface PluralsightDisplayCourse {
  key: string;
  title: string;
  coverUrl: string | null;
  facts: string[];
  folder: string;
  issue: string | null;
}

/** Keep malformed history entries visible while ensuring they cannot be started. */
export function displayPluralsightCourses(
  courses: PluralsightCourse[],
  missingIdMessage: string,
): PluralsightDisplayCourse[] {
  return courses.map((course, index) => ({
    key: course.id ?? `invalid-history-row-${index}`,
    title: course.title,
    coverUrl: course.imageUrl,
    facts: course.level ? [course.level] : [],
    folder: course.folder,
    issue: course.issue ?? (course.id ? null : missingIdMessage),
  }));
}
