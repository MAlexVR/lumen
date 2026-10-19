import { describe, expect, it } from "vitest";
import { formatTime, resumeSeek, shouldSavePosition } from "./time";

describe("formatTime", () => {
  it("formats minutes and seconds", () => {
    expect(formatTime(0)).toBe("0:00");
    expect(formatTime(65)).toBe("1:05");
    expect(formatTime(599.9)).toBe("9:59");
  });

  it("adds hours when needed", () => {
    expect(formatTime(3600)).toBe("1:00:00");
    expect(formatTime(3725)).toBe("1:02:05");
  });

  it("clamps invalid input to zero", () => {
    expect(formatTime(-3)).toBe("0:00");
    expect(formatTime(Number.NaN)).toBe("0:00");
    expect(formatTime(Number.POSITIVE_INFINITY)).toBe("0:00");
  });
});

describe("resumeSeek", () => {
  it("resumes a saved position inside the video", () => {
    expect(resumeSeek(120, 600)).toBe(120);
  });

  it("ignores the first five seconds", () => {
    expect(resumeSeek(4, 600)).toBe(0);
    expect(resumeSeek(5, 600)).toBe(5);
  });

  it("ignores the last five seconds", () => {
    expect(resumeSeek(596, 600)).toBe(0);
    expect(resumeSeek(594, 600)).toBe(594);
  });

  it("ignores unknown duration", () => {
    expect(resumeSeek(120, Number.NaN)).toBe(0);
    expect(resumeSeek(120, 0)).toBe(0);
  });
});

describe("shouldSavePosition", () => {
  it("saves at most every five seconds unless forced", () => {
    expect(shouldSavePosition(10_000, 6_000, false)).toBe(false);
    expect(shouldSavePosition(11_000, 6_000, false)).toBe(true);
    expect(shouldSavePosition(6_001, 6_000, true)).toBe(true);
  });
});

import { formatIsoDate } from "./time";

describe("formatIsoDate", () => {
  it("formats a YYYY-MM-DD date for the locale without shifting the day", () => {
    expect(formatIsoDate("2024-03-18", "es")).toBe("18 mar 2024");
    expect(formatIsoDate("2024-03-18", "en")).toBe("Mar 18, 2024");
  });

  it("returns null for missing or malformed dates", () => {
    expect(formatIsoDate(null, "es")).toBeNull();
    expect(formatIsoDate("2024-13-40", "es")).toBeNull();
    expect(formatIsoDate("yesterday", "es")).toBeNull();
  });
});
