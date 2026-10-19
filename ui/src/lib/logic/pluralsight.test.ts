import { describe, expect, it } from "vitest";
import type { PluralsightCourse } from "../types";
import { displayPluralsightCourses } from "./pluralsight";

const rows: PluralsightCourse[] = [
  { id: "c-1", title: "Valid course", level: "Intermediate", imageUrl: null, folder: "Valid-abc", issue: null },
  { id: null, title: "Malformed row", level: null, imageUrl: null, folder: "Malformed-def", issue: "Missing course identifier" },
];

describe("Pluralsight history display", () => {
  it("preserves malformed rows and makes them non-downloadable", () => {
    expect(displayPluralsightCourses(rows, "Cannot identify this course")).toEqual([
      { key: "c-1", title: "Valid course", coverUrl: null, facts: ["Intermediate"], folder: "Valid-abc", issue: null },
      { key: "invalid-history-row-1", title: "Malformed row", coverUrl: null, facts: [], folder: "Malformed-def", issue: "Missing course identifier" },
    ]);
  });

  it("provides a useful explanation when a malformed row has no backend issue", () => {
    expect(displayPluralsightCourses([{ ...rows[1], issue: null }], "Cannot identify this course")[0].issue)
      .toBe("Cannot identify this course");
  });
});
