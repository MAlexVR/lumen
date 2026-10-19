import { describe, expect, it } from "vitest";
import { previewFace, previewSrc, previewTime } from "./preview";

const lesson = (video: string | null) => ({ video, hasVideo: video !== null });

describe("previewTime", () => {
  it("uses the resume position when it is worth resuming", () => {
    expect(previewTime(95, 600)).toBe(95);
  });

  it("uses about 10% of the duration without a usable resume position", () => {
    expect(previewTime(0, 600)).toBe(60);
    expect(previewTime(3, 600)).toBe(60);
    expect(previewTime(598, 600)).toBe(60);
  });

  it("returns 0 when the duration is unknown or invalid", () => {
    expect(previewTime(95, Number.NaN)).toBe(0);
    expect(previewTime(95, 0)).toBe(0);
    expect(previewTime(95, Number.POSITIVE_INFINITY)).toBe(0);
  });
});

describe("previewSrc", () => {
  it("builds a media server URL with every path segment encoded", () => {
    expect(previewSrc("http://127.0.0.1:9/m/tok", lesson("Skool/G/01 - A/01 - S/01 - 👋 Hola?/v #1.mp4"))).toBe(
      "http://127.0.0.1:9/m/tok/Skool/G/01%20-%20A/01%20-%20S/01%20-%20%F0%9F%91%8B%20Hola%3F/v%20%231.mp4",
    );
  });

  it("is null without a media server or without a video", () => {
    expect(previewSrc("", lesson("Skool/G/v.mp4"))).toBeNull();
    expect(previewSrc("http://127.0.0.1:9/m/tok", lesson(null))).toBeNull();
    expect(previewSrc("http://127.0.0.1:9/m/tok", { video: "x.mp4", hasVideo: false })).toBeNull();
  });
});

describe("previewFace", () => {
  it("shows the poster until a frame is ready", () => {
    expect(previewFace({ src: "u", ready: false, failed: false })).toBe("poster");
  });

  it("shows the video frame once ready", () => {
    expect(previewFace({ src: "u", ready: true, failed: false })).toBe("video");
  });

  it("falls back to the poster on error or without a source", () => {
    expect(previewFace({ src: "u", ready: true, failed: true })).toBe("poster");
    expect(previewFace({ src: null, ready: true, failed: false })).toBe("poster");
  });
});
