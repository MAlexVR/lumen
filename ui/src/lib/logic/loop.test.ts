import { describe, expect, it } from "vitest";
import { afterEnd } from "./loop";

describe("afterEnd", () => {
  it("replays the video once it has played to its end while looping", () => {
    expect(afterEnd(true)).toBe("replay");
  });

  it("moves on (autoplay-next logic) when not looping", () => {
    expect(afterEnd(false)).toBe("advance");
  });
});
