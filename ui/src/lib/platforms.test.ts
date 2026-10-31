import { describe, expect, it } from "vitest";
import { gridMove, PLATFORMS, platformById } from "./platforms";

describe("platform registry", () => {
  it("lists the platforms Lumen offers, available ones first", () => {
    expect(PLATFORMS.map((p) => [p.id, p.name])).toEqual([
      ["hotmart", "Hotmart"],
      ["skool", "Skool"],
      ["pluralsight", "Pluralsight"],
      ["goldenlife", "Golden Life"],
      ["harbiz", "Harbiz"],
      ["youtube", "YouTube"],
      ["udemy", "Udemy"],
    ]);
  });

  it("marks only platforms with a backend as available", () => {
    expect(PLATFORMS.filter((p) => p.status === "available").map((p) => p.id)).toEqual(["hotmart", "skool", "pluralsight", "goldenlife", "harbiz", "youtube", "udemy"]);
  });

  it("routes each platform under downloads", () => {
    for (const p of PLATFORMS) expect(p.route).toEqual({ name: "downloads", platform: p.id });
  });

  it("gives every platform a logo and a hex stage background", () => {
    for (const p of PLATFORMS) {
      expect(p.logo).toMatch(/\.(webp|svg)$|^data:image\/svg\+xml/);
      expect(p.stageBg).toMatch(/^#[0-9a-f]{6}$/i);
    }
  });

  it("finds platforms by id", () => {
    expect(platformById("skool")?.name).toBe("Skool");
    expect(platformById("nope")).toBeUndefined();
    for (const removed of ["gumroad", "kiwify", "teachable", "kajabi", "greatcourses", "masterclass", "thinkific"])
      expect(platformById(removed)).toBeUndefined();
    expect(platformById(null)).toBeUndefined();
  });
});

describe("gridMove", () => {
  // 10 tiles in 3 columns: rows [0,1,2] [3,4,5] [6,7,8] [9]
  it("moves left and right within bounds", () => {
    expect(gridMove(4, "ArrowRight", 3, 10)).toBe(5);
    expect(gridMove(4, "ArrowLeft", 3, 10)).toBe(3);
    expect(gridMove(0, "ArrowLeft", 3, 10)).toBe(0);
    expect(gridMove(9, "ArrowRight", 3, 10)).toBe(9);
  });

  it("moves up and down by a row and clamps to the last tile", () => {
    expect(gridMove(1, "ArrowDown", 3, 10)).toBe(4);
    expect(gridMove(4, "ArrowUp", 3, 10)).toBe(1);
    expect(gridMove(1, "ArrowUp", 3, 10)).toBe(1);
    expect(gridMove(8, "ArrowDown", 3, 10)).toBe(9);
  });

  it("jumps to the ends with Home and End", () => {
    expect(gridMove(5, "Home", 3, 10)).toBe(0);
    expect(gridMove(5, "End", 3, 10)).toBe(9);
  });

  it("returns null for other keys", () => {
    expect(gridMove(5, "Enter", 3, 10)).toBeNull();
  });
});

describe("platform marks", () => {
  it("every platform has a mark with a source, a background and a sane scale", () => {
    for (const x of PLATFORMS) {
      expect(x.mark.src, x.id).toBeTruthy();
      expect(x.mark.bg, x.id).toMatch(/^#[0-9a-f]{6}$/i);
      expect(x.mark.scale, x.id).toBeGreaterThanOrEqual(0.5);
      expect(x.mark.scale, x.id).toBeLessThanOrEqual(4);
    }
  });
});
