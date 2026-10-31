import { describe, expect, it } from "vitest";
import type { HarbizCourse } from "../types";
import { displayHarbizCourses, type HarbizLabels } from "./harbiz";

const labels: HarbizLabels = {
  workouts: (n) => `${n} workouts`,
  pending: (n) => `${n} new to download`,
  upToDate: "Up to date",
  range: (first, last) => `${first} to ${last}`,
  empty: "Nothing on the agenda",
};

const course = (over: Partial<HarbizCourse> = {}): HarbizCourse => ({
  id: "harbiz",
  title: "Harbiz Training",
  folder: "Harbiz Training",
  workouts: 66,
  downloaded: 62,
  pending: 4,
  firstDate: "2026-06-01",
  lastDate: "2026-10-31",
  photoCheckins: 5,
  ...over,
});

describe("Harbiz course display", () => {
  it("shows how many workouts there are, how many are new, and the date range", () => {
    expect(displayHarbizCourses([course()], labels)).toEqual([
      {
        key: "harbiz",
        title: "Harbiz Training",
        coverUrl: null,
        facts: ["66 workouts", "4 new to download", "2026-06-01 to 2026-10-31"],
        folder: "Harbiz Training",
        issue: null,
      },
    ]);
  });

  it("says up to date when nothing is pending", () => {
    expect(displayHarbizCourses([course({ downloaded: 66, pending: 0 })], labels)[0].facts[1]).toBe("Up to date");
  });

  it("flags an empty agenda instead of offering a download", () => {
    const [row] = displayHarbizCourses([course({ workouts: 0, downloaded: 0, pending: 0, firstDate: null, lastDate: null })], labels);
    expect(row.issue).toBe("Nothing on the agenda");
    expect(row.facts).toEqual(["0 workouts"]);
  });
});
