import type { HarbizCourse } from "../types";

export interface HarbizDisplayCourse {
  key: string;
  title: string;
  coverUrl: string | null;
  facts: string[];
  folder: string;
  issue: string | null;
}

/** Wording is passed in so this stays a pure function of the data. */
export interface HarbizLabels {
  workouts(n: number): string;
  pending(n: number): string;
  upToDate: string;
  range(first: string, last: string): string;
  empty: string;
}

/** The agenda is one course; its facts tell what is new so the owner knows an update is worth running. */
export function displayHarbizCourses(courses: HarbizCourse[], labels: HarbizLabels): HarbizDisplayCourse[] {
  return courses.map((course) => {
    const facts = [labels.workouts(course.workouts)];
    if (course.workouts > 0) {
      facts.push(course.pending > 0 ? labels.pending(course.pending) : labels.upToDate);
      if (course.firstDate && course.lastDate) facts.push(labels.range(course.firstDate, course.lastDate));
    }
    return {
      key: course.id,
      title: course.title,
      coverUrl: null,
      facts,
      folder: course.folder,
      issue: course.workouts === 0 ? labels.empty : null,
    };
  });
}
