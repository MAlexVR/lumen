import { describe, expect, it } from "vitest";
import { displayTitle, stripEmoji } from "./title";

describe("displayTitle", () => {
  it.each([
    ["HÁBITOS DE LOS QUE AVANZAN", "Hábitos de los que Avanzan"],
    ["EL ARTE DE ORDENAR PDFs", "El Arte de Ordenar PDFs"],
    ["COMIENZA TU RECORRIDO", "Comienza tu Recorrido"],
    ["CONOCE LAS HERRAMIENTAS BÁSICAS", "Conoce las Herramientas Básicas"],
    ["LA MESA DE EDICIÓN", "La Mesa de Edición"],
    ["EXPERIMENTOS UX.AI", "Experimentos UX.AI"],
    ["ACADEMIA AURORA", "Academia Aurora"],
    ["TALLER DE IA PARA NEGOCIOS", "Taller de IA para Negocios"],
    ["COMPÓN CON DAW", "Compón con DAW"],
    ["MÓDULO II: LA OFERTA", "Módulo II: La Oferta"],
    ["CLASE 3 Y 4 EN VIVO", "Clase 3 y 4 en Vivo"],
    ["¿QUÉ ES UN EMBUDO?", "¿Qué Es un Embudo?"],
    ["ESTRATEGIA 80/20", "Estrategia 80/20"],
  ])("normalizes %j", (input, expected) => {
    expect(displayTitle(input)).toBe(expected);
  });

  it("leaves mixed-case titles untouched", () => {
    expect(displayTitle("Informe Trimestral KPI")).toBe("Informe Trimestral KPI");
    expect(displayTitle("Compón con DAW´s")).toBe("Compón con DAW´s");
    expect(displayTitle("Compón con DAW´s 2")).toBe("Compón con DAW´s 2");
    expect(displayTitle("Modo Cabaña")).toBe("Modo Cabaña");
  });

  it("capitalizes the first word even when it is a connector", () => {
    expect(displayTitle("DE CERO A CIEN")).toBe("De Cero a Cien");
    expect(displayTitle("👋 EL INICIO")).toBe("El Inicio");
  });

  it("keeps vowel-less acronyms and words with digits", () => {
    expect(displayTitle("GUÍA CRM Y KPI 2024")).toBe("Guía CRM y KPI 2024");
    expect(displayTitle("GUÍA RÁPIDA UX")).toBe("Guía Rápida UX");
  });

  it("handles empty and whitespace", () => {
    expect(displayTitle("")).toBe("");
    expect(displayTitle("   ")).toBe("   ");
  });
});

describe("stripEmoji", () => {
  it.each([
    ["👋 Bienvenida", "Bienvenida"],
    ["🚀 Growth Lab ™", "Growth Lab ™"],
    ["👨‍💻 Curso de código", "Curso de código"],
    ["❤️ Amor propio", "Amor propio"],
    ["Hecho ✔️", "Hecho"],
    ["👍🏽 Buen trabajo", "Buen trabajo"],
    ["Paso 1️⃣ inicial", "Paso inicial"],
    ["🇪🇸 España", "España"],
    ["🏴󠁧󠁢󠁳󠁣󠁴󠁿 Escocia", "Escocia"],
    ["Curso ✅  Ñandú 2024", "Curso Ñandú 2024"],
    ["📜 Reglas de la comunidad", "Reglas de la comunidad"],
  ])("strips %j", (input, expected) => {
    expect(stripEmoji(input)).toBe(expected);
  });

  it("keeps text symbols that are not emoji", () => {
    expect(stripEmoji("Precio © 2024 → ★ #1 • 50% · A&B")).toBe("Precio © 2024 → ★ #1 • 50% · A&B");
    expect(stripEmoji("Compón con DAW´s")).toBe("Compón con DAW´s");
  });

  it("keeps accents and Spanish punctuation", () => {
    expect(stripEmoji("¿Qué es? ¡Ñandú! Acción, pingüino")).toBe("¿Qué es? ¡Ñandú! Acción, pingüino");
  });

  it("trims and collapses spaces", () => {
    expect(stripEmoji("  🔥  Lanzamiento   rápido  🔥 ")).toBe("Lanzamiento rápido");
  });

  it("falls back to the original when nothing but emoji remains", () => {
    expect(stripEmoji("🔥🔥")).toBe("🔥🔥");
    expect(stripEmoji("")).toBe("");
  });

  it("feeds the title normalizer", () => {
    expect(displayTitle("👋 Bienvenida")).toBe("Bienvenida");
    expect(displayTitle("🚀 HÁBITOS DE LOS QUE AVANZAN")).toBe("Hábitos de los que Avanzan");
  });
});
