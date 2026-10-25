import { describe, expect, it } from "vitest";
import type { GoldenlifeCourse } from "../types";
import { canSubmitCredentials, displayGoldenlifeCourses } from "./goldenlife";

const rows: GoldenlifeCourse[] = [
  { id: "7", title: "Creator Basics", shelf: "UGC", imageUrl: null, folder: "Creator Basics", issue: null },
  { id: null, title: "Alias row", shelf: null, imageUrl: null, folder: "Alias row", issue: "Course without modules" },
];

describe("Golden Life course display", () => {
  it("shows the shelf as the course fact and keeps broken rows visible", () => {
    expect(displayGoldenlifeCourses(rows, "Cannot identify this course")).toEqual([
      { key: "7", title: "Creator Basics", coverUrl: null, facts: ["UGC"], folder: "Creator Basics", issue: null },
      { key: "invalid-course-row-1", title: "Alias row", coverUrl: null, facts: [], folder: "Alias row", issue: "Course without modules" },
    ]);
  });

  it("explains a row without id when the backend gave no issue", () => {
    expect(displayGoldenlifeCourses([{ ...rows[1], issue: null }], "Cannot identify this course")[0].issue).toBe("Cannot identify this course");
  });
});

describe("canSubmitCredentials", () => {
  it("needs a plausible email and a non-empty password", () => {
    expect(canSubmitCredentials("ana@example.com", "secret")).toBe(true);
    expect(canSubmitCredentials("  ana@example.com ", "secret")).toBe(true);
    expect(canSubmitCredentials("", "secret")).toBe(false);
    expect(canSubmitCredentials("not-an-email", "secret")).toBe(false);
    expect(canSubmitCredentials("ana@example.com", "")).toBe(false);
  });
});
