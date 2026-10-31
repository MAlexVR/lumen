import { describe, expect, it } from "vitest";
import {
  completeSet,
  elapsedMs,
  formatElapsed,
  newSession,
  resetSession,
  setProgress,
  shouldHold,
  pauseTimer,
  resumeTimer,
  toggleTimer,
  undoSet,
} from "./workoutSession";

describe("timer", () => {
  it("does not run until the first video plays, then counts from that moment", () => {
    const s = newSession();
    expect(elapsedMs(s, 5_000)).toBe(0);
    const started = resumeTimer(s, 10_000);
    expect(elapsedMs(started, 10_000)).toBe(0);
    expect(elapsedMs(started, 75_000)).toBe(65_000);
  });

  it("a video starting while the timer already runs (the next exercise) changes nothing", () => {
    const s = resumeTimer(newSession(), 1_000);
    expect(resumeTimer(s, 50_000)).toEqual(s);
  });

  it("pausing the video stops the clock and playing again resumes it without counting the pause", () => {
    let s = resumeTimer(newSession(), 0);
    s = pauseTimer(s, 30_000);
    expect(elapsedMs(s, 90_000)).toBe(30_000);
    s = resumeTimer(s, 100_000);
    expect(elapsedMs(s, 120_000)).toBe(50_000);
  });

  it("pausing twice, or before anything played, changes nothing", () => {
    expect(pauseTimer(newSession(), 5_000)).toEqual(newSession());
    const paused = pauseTimer(resumeTimer(newSession(), 0), 10_000);
    expect(pauseTimer(paused, 20_000)).toEqual(paused);
  });

  it("the click on the timer pauses or resumes it by hand", () => {
    let s = resumeTimer(newSession(), 0);
    s = toggleTimer(s, 30_000);
    expect(elapsedMs(s, 90_000)).toBe(30_000);
    s = toggleTimer(s, 100_000);
    expect(elapsedMs(s, 120_000)).toBe(50_000);
  });

  it("toggling before the workout started does nothing", () => {
    expect(toggleTimer(newSession(), 1_000)).toEqual(newSession());
  });

  it("formats minutes and hours", () => {
    expect(formatElapsed(0)).toBe("0:00");
    expect(formatElapsed(65_000)).toBe("1:05");
    expect(formatElapsed(59_999)).toBe("0:59");
    expect(formatElapsed(3_600_000 + 5 * 60_000 + 7_000)).toBe("1:05:07");
  });
});

describe("sets", () => {
  it("starts on set 1 and moves to the next each time one is marked done", () => {
    let s = newSession();
    expect(setProgress(s, 3, 3)).toEqual({ done: 0, current: 1, total: 3, finished: false });
    s = completeSet(s, 3, 3);
    expect(setProgress(s, 3, 3)).toEqual({ done: 1, current: 2, total: 3, finished: false });
  });

  it("finishes after the last set and cannot go past it", () => {
    let s = newSession();
    for (let i = 0; i < 5; i++) s = completeSet(s, 1, 2);
    expect(setProgress(s, 1, 2)).toEqual({ done: 2, current: 2, total: 2, finished: true });
  });

  it("keeps each exercise's sets apart", () => {
    let s = completeSet(newSession(), 1, 2);
    s = completeSet(s, 2, 3);
    s = completeSet(s, 2, 3);
    expect(setProgress(s, 1, 2).done).toBe(1);
    expect(setProgress(s, 2, 3).done).toBe(2);
  });

  it("undo takes back the last set and never goes below zero", () => {
    let s = completeSet(newSession(), 1, 3);
    s = undoSet(s, 1);
    s = undoSet(s, 1);
    expect(setProgress(s, 1, 3).done).toBe(0);
  });

  it("without a known total it just counts and never finishes", () => {
    let s = completeSet(newSession(), 4, null);
    s = completeSet(s, 4, null);
    expect(setProgress(s, 4, null)).toEqual({ done: 2, current: 3, total: null, finished: false });
  });
});

describe("resetSession", () => {
  it("clears the timer and every set", () => {
    let s = resumeTimer(newSession(), 0);
    s = completeSet(s, 1, 2);
    expect(resetSession()).toEqual(newSession());
    expect(elapsedMs(resetSession(), 99_000)).toBe(0);
    expect(setProgress(resetSession(), 1, 2).done).toBe(0);
    void s;
  });
});

describe("shouldHold", () => {
  it("holds on an exercise while it still has sets left", () => {
    expect(shouldHold({ done: 1, current: 2, total: 3, finished: false })).toBe(true);
  });

  it("lets the video move on once every set is done", () => {
    expect(shouldHold({ done: 3, current: 3, total: 3, finished: true })).toBe(false);
  });

  it("never holds when there is no plan or the number of sets is unknown", () => {
    expect(shouldHold(null)).toBe(false);
    expect(shouldHold({ done: 0, current: 1, total: null, finished: false })).toBe(false);
  });
});
