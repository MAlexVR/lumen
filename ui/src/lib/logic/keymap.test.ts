import { describe, expect, it } from "vitest";
import { actionForKey, nextSpeed, SPEEDS, type KeyInput } from "./keymap";

const k = (key: string, extra: Partial<KeyInput> = {}): KeyInput => ({
  key,
  shiftKey: false,
  ctrlKey: false,
  altKey: false,
  metaKey: false,
  editable: false,
  ...extra,
});

describe("actionForKey", () => {
  it.each([
    [" ", { type: "toggle-play" }],
    ["k", { type: "toggle-play" }],
    ["K", { type: "toggle-play" }],
    ["j", { type: "seek", by: -10 }],
    ["l", { type: "seek", by: 10 }],
    ["ArrowLeft", { type: "seek", by: -5 }],
    ["ArrowRight", { type: "seek", by: 5 }],
    ["ArrowUp", { type: "volume", by: 0.05 }],
    ["ArrowDown", { type: "volume", by: -0.05 }],
    ["m", { type: "toggle-mute" }],
    ["f", { type: "toggle-fullscreen" }],
    ["t", { type: "toggle-theatre" }],
    ["n", { type: "next" }],
    ["p", { type: "prev" }],
    ["0", { type: "restart" }],
    ["Home", { type: "restart" }],
  ])("maps %j", (key, action) => {
    expect(actionForKey(k(key))).toEqual(action);
  });

  it("maps shift+< and shift+> to speed changes", () => {
    expect(actionForKey(k("<", { shiftKey: true }))).toEqual({ type: "speed", dir: -1 });
    expect(actionForKey(k(">", { shiftKey: true }))).toEqual({ type: "speed", dir: 1 });
    expect(actionForKey(k(",", { shiftKey: true }))).toEqual({ type: "speed", dir: -1 });
    expect(actionForKey(k(".", { shiftKey: true }))).toEqual({ type: "speed", dir: 1 });
  });

  it("does nothing while typing in a field", () => {
    expect(actionForKey(k("k", { editable: true }))).toBeNull();
  });

  it("leaves modifier shortcuts to the system", () => {
    expect(actionForKey(k("f", { ctrlKey: true }))).toBeNull();
    expect(actionForKey(k("ArrowLeft", { altKey: true }))).toBeNull();
    expect(actionForKey(k("l", { metaKey: true }))).toBeNull();
  });

  it("ignores unmapped keys", () => {
    expect(actionForKey(k("x"))).toBeNull();
  });
});

describe("nextSpeed", () => {
  it("steps through the supported speeds and clamps at the ends", () => {
    expect(SPEEDS[0]).toBe(0.75);
    expect(SPEEDS[SPEEDS.length - 1]).toBe(2);
    expect(nextSpeed(1, 1)).toBe(1.25);
    expect(nextSpeed(1, -1)).toBe(0.75);
    expect(nextSpeed(2, 1)).toBe(2);
    expect(nextSpeed(0.75, -1)).toBe(0.75);
  });

  it("snaps an unknown speed to the nearest step first", () => {
    expect(nextSpeed(1.1, 1)).toBe(1.25);
  });
});
