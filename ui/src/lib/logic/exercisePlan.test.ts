import { describe, expect, it } from "vitest";
import type { ExercisePlan } from "../types";
import { exerciseDisplayName, exerciseNumber, planFor, targetLabel } from "./exercisePlan";

const plans: ExercisePlan[] = [
  { number: 1, name: "Bodyweight squat", sets: 2, target: "10", rest: 30 },
  { number: 3, name: "Walking lunge", sets: 3, target: "45 s", rest: null },
];

describe("exerciseNumber", () => {
  it("reads the NN of a video file name or path", () => {
    expect(exerciseNumber("Harbiz/T/2026-06/c/03. Walking lunge.mp4")).toBe(3);
    expect(exerciseNumber("10. Lunge.mkv")).toBe(10);
  });

  it("is null when the file has no number", () => {
    expect(exerciseNumber("Harbiz/T/c/Intro.mp4")).toBeNull();
  });
});

describe("planFor", () => {
  it("finds the plan of the exercise a video belongs to", () => {
    expect(planFor(plans, "x/03. Walking lunge.mp4")?.sets).toBe(3);
  });

  it("is null when the video has no plan (a gap in the numbers, or a non-workout lesson)", () => {
    expect(planFor(plans, "x/02. Missing.mp4")).toBeNull();
    expect(planFor([], "x/01. Bodyweight squat.mp4")).toBeNull();
    expect(planFor(plans, "x/Intro.mp4")).toBeNull();
  });
});

describe("targetLabel", () => {
  it("adds the repetitions word to a plain number", () => {
    expect(targetLabel("10", "reps")).toBe("10 reps");
  });

  it("keeps anything else as the coach wrote it", () => {
    expect(targetLabel("45 s", "reps")).toBe("45 s");
    expect(targetLabel("8-12", "reps")).toBe("8-12 reps");
  });

  it("is empty when there is no target", () => {
    expect(targetLabel("", "reps")).toBe("");
  });
});

describe("exerciseDisplayName", () => {
  it("prefers the name the coach gave the exercise, with doubled spaces collapsed", () => {
    const plan: ExercisePlan = { number: 3, name: "Walking  lunge", sets: 2, target: "10", rest: 30 };
    expect(exerciseDisplayName(plan, "Walking lunge")).toBe("Walking lunge");
  });

  it("falls back to the title taken from the video file", () => {
    expect(exerciseDisplayName(null, "Bodyweight squat")).toBe("Bodyweight squat");
    expect(exerciseDisplayName({ number: 1, name: "  ", sets: null, target: "", rest: null }, "Bodyweight squat")).toBe("Bodyweight squat");
  });

  it("is empty when there is nothing to show", () => {
    expect(exerciseDisplayName(null, "")).toBe("");
  });
});
