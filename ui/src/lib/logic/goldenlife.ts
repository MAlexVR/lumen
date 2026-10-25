import type { GoldenlifeCourse } from "../types";

export interface GoldenlifeDisplayCourse {
  key: string;
  title: string;
  coverUrl: string | null;
  facts: string[];
  folder: string;
  issue: string | null;
}

/** The academy section is the course fact; rows without an id stay visible but cannot start. */
export function displayGoldenlifeCourses(courses: GoldenlifeCourse[], missingIdMessage: string): GoldenlifeDisplayCourse[] {
  return courses.map((course, index) => ({
    key: course.id ?? `invalid-course-row-${index}`,
    title: course.title,
    coverUrl: course.imageUrl,
    facts: course.shelf ? [course.shelf] : [],
    folder: course.folder,
    issue: course.issue ?? (course.id ? null : missingIdMessage),
  }));
}

/** Cheap client-side gate; the academy is the one that decides if the credentials are right. */
export function canSubmitCredentials(email: string, password: string): boolean {
  return /^\S+@\S+\.\S+$/.test(email.trim()) && password.length > 0;
}
