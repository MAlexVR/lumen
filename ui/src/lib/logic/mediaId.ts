// The video element is reused when the player moves to another exercise or
// lesson, so every save and "watched" it reports carries the identity of the
// video that was really loaded, not whatever the screen shows by then.

export interface MediaRef {
  key: string;
  index: number;
  count: number;
}

export const encodeMedia = (key: string, index: number, count: number): string => `${index}/${count}|${key}`;

export function decodeMedia(id: string): MediaRef | null {
  const match = /^(\d+)\/(\d+)\|(.+)$/s.exec(id);
  return match ? { index: Number(match[1]), count: Number(match[2]), key: match[3] } : null;
}
