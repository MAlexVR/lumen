import { describe, expect, it } from "vitest";
import { initials, MOTIFS, posterSeed } from "./poster";

describe("initials", () => {
  it("takes the first letters of the first two meaningful words", () => {
    expect(initials("Modo Cabaña")).toBe("MC");
    expect(initials("La mesa de edición")).toBe("ME");
    expect(initials("EL ARTE DE ORDENAR PDFs")).toBe("AO");
  });

  it("ignores emoji and punctuation", () => {
    expect(initials("👋 Bienvenida")).toBe("B");
    expect(initials("Compón con DAW´s")).toBe("CD");
  });

  it("uses a placeholder for empty titles", () => {
    expect(initials("  ")).toBe("·");
  });
});

// Invented course titles.
const SAMPLE = [
  "COMIENZA TU RECORRIDO",
  "HÁBITOS DE LOS QUE AVANZAN",
  "CONOCE LAS HERRAMIENTAS BÁSICAS",
  "LA MESA DE EDICIÓN",
  "EL ARTE DE ORDENAR PDFs",
  "EXPERIMENTOS UX.AI",
  "FaroIA",
  "Laboratorio de Diseño Aplicado a Proyectos Creativos",
  "Compón con DAW´s",
  "Modo Cabaña",
  "Guía Rápida UX",
];

describe("posterSeed", () => {
  it("offers six tone-on-tone motifs", () => {
    expect(MOTIFS).toEqual(["arcs", "rays", "dots", "diagonals", "horizon", "orbit"]);
  });

  it("is deterministic for a title", () => {
    for (const t of SAMPLE) expect(posterSeed(t)).toEqual(posterSeed(t));
  });

  it("keeps every parameter in range", () => {
    for (const t of SAMPLE) {
      const s = posterSeed(t);
      expect(MOTIFS).toContain(s.motif);
      expect(["lumen", "moon"]).toContain(s.glow);
      for (const v of [s.x, s.y]) {
        expect(v).toBeGreaterThanOrEqual(0);
        expect(v).toBeLessThanOrEqual(1);
      }
      expect(s.scale).toBeGreaterThanOrEqual(0.8);
      expect(s.scale).toBeLessThanOrEqual(1.25);
    }
  });

  it("spreads the sample library across motifs", () => {
    const motifs = new Set(SAMPLE.map((t) => posterSeed(t).motif));
    expect(motifs.size).toBeGreaterThanOrEqual(5);
    const counts = new Map<string, number>();
    for (const t of SAMPLE) counts.set(posterSeed(t).motif, (counts.get(posterSeed(t).motif) ?? 0) + 1);
    expect(Math.max(...counts.values())).toBeLessThanOrEqual(3);
  });

  it("does not repeat the same motif for neighbouring courses too often", () => {
    const seq = SAMPLE.map((t) => posterSeed(t).motif);
    const repeats = seq.filter((m, i) => i > 0 && m === seq[i - 1]).length;
    expect(repeats).toBeLessThanOrEqual(2);
  });
});
