import { describe, expect, it } from "vitest";
import { decodeMedia, encodeMedia } from "./mediaId";

describe("mediaId", () => {
  it("round-trips the lesson, the video index and how many videos the lesson has", () => {
    expect(decodeMedia(encodeMedia("harbiz:1-T/0-2026-06/0-2026-06-01 - Legs", 2, 5))).toEqual({
      key: "harbiz:1-T/0-2026-06/0-2026-06-01 - Legs",
      index: 2,
      count: 5,
    });
  });

  it("keeps lesson keys that contain the separator intact", () => {
    expect(decodeMedia(encodeMedia("a|b|c", 0, 1))).toEqual({ key: "a|b|c", index: 0, count: 1 });
  });

  it("says whether the video is the last of its lesson", () => {
    expect(decodeMedia(encodeMedia("k", 4, 5))?.index).toBe(4);
    const m = decodeMedia(encodeMedia("k", 4, 5));
    expect(m && m.index === m.count - 1).toBe(true);
  });

  it("returns null for an id it did not produce", () => {
    expect(decodeMedia("")).toBeNull();
    expect(decodeMedia("garbage")).toBeNull();
    expect(decodeMedia("x/y|key")).toBeNull();
  });
});
