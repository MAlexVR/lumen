// Deterministic inputs for the generated typographic poster (real stills come in S3).

export const MOTIFS = ["arcs", "rays", "dots", "diagonals", "horizon", "orbit"] as const;
export type Motif = (typeof MOTIFS)[number];

export interface PosterSeed {
  motif: Motif;
  /** Motif anchor, 0..1 of width and height. */
  x: number;
  y: number;
  /** Motif scale, 0.8..1.25. */
  scale: number;
  /** Which accent tints the faint glow behind the motif. */
  glow: "lumen" | "moon";
}

const STOP = new Set(["la", "el", "los", "las", "de", "del", "y", "en", "con", "a", "the", "of", "and", "to", "por"]);

export function initials(title: string): string {
  const words = title
    .split(/\s+/)
    .map((w) => w.replace(/[^\p{L}\p{N}]/gu, ""))
    .filter(Boolean);
  const meaningful = words.filter((w) => !STOP.has(w.toLowerCase()));
  const pick = (meaningful.length ? meaningful : words).slice(0, 2);
  const out = pick.map((w) => w[0].toUpperCase()).join("");
  return out || "·";
}

/** FNV-1a, then a murmur-style finalizer so nearby titles diverge. */
function hash(s: string): number {
  let h = 2166136261;
  for (const ch of s) {
    h ^= ch.codePointAt(0) ?? 0;
    h = Math.imul(h, 16777619);
  }
  h ^= h >>> 16;
  h = Math.imul(h, 0x85ebca6b);
  h ^= h >>> 13;
  h = Math.imul(h, 0xc2b2ae35);
  h ^= h >>> 16;
  return h >>> 0;
}

const unit = (h: number, shift: number): number => ((h >>> shift) & 0xff) / 255;

export function posterSeed(title: string): PosterSeed {
  const h = hash(title);
  return {
    motif: MOTIFS[(h >>> 17) % MOTIFS.length],
    x: 0.2 + unit(h, 5) * 0.6,
    y: 0.15 + unit(h, 13) * 0.55,
    scale: 0.8 + unit(h, 21) * 0.45,
    glow: (h >>> 29) % 3 === 0 ? "moon" : "lumen",
  };
}
