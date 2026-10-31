// Player keyboard map (YouTube-like). Pure so it can be tested without a DOM.

export interface KeyInput {
  key: string;
  shiftKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  metaKey: boolean;
  /** True when focus is in a text field, where keys must type. */
  editable: boolean;
}

export type PlayerAction =
  | { type: "toggle-play" }
  | { type: "restart" }
  | { type: "toggle-loop" }
  | { type: "seek"; by: number }
  | { type: "volume"; by: number }
  | { type: "toggle-mute" }
  | { type: "toggle-fullscreen" }
  | { type: "toggle-theatre" }
  | { type: "speed"; dir: 1 | -1 }
  | { type: "next" }
  | { type: "prev" };

export const SPEEDS: readonly number[] = [0.75, 1, 1.25, 1.5, 1.75, 2];

const PLAIN: Record<string, PlayerAction> = {
  " ": { type: "toggle-play" },
  k: { type: "toggle-play" },
  "0": { type: "restart" },
  r: { type: "toggle-loop" },
  home: { type: "restart" },
  j: { type: "seek", by: -10 },
  l: { type: "seek", by: 10 },
  arrowleft: { type: "seek", by: -5 },
  arrowright: { type: "seek", by: 5 },
  arrowup: { type: "volume", by: 0.05 },
  arrowdown: { type: "volume", by: -0.05 },
  m: { type: "toggle-mute" },
  f: { type: "toggle-fullscreen" },
  t: { type: "toggle-theatre" },
  n: { type: "next" },
  p: { type: "prev" },
};

export function actionForKey(input: KeyInput): PlayerAction | null {
  if (input.editable || input.ctrlKey || input.altKey || input.metaKey) return null;
  if (input.shiftKey && (input.key === "<" || input.key === ",")) return { type: "speed", dir: -1 };
  if (input.shiftKey && (input.key === ">" || input.key === ".")) return { type: "speed", dir: 1 };
  return PLAIN[input.key.toLowerCase()] ?? null;
}

/** Next supported speed in `dir`; an off-grid speed moves to the nearest step beyond it. */
export function nextSpeed(current: number, dir: 1 | -1): number {
  const candidates = dir > 0 ? SPEEDS.filter((s) => s > current) : SPEEDS.filter((s) => s < current).reverse();
  return candidates[0] ?? (dir > 0 ? SPEEDS[SPEEDS.length - 1] : SPEEDS[0]);
}
