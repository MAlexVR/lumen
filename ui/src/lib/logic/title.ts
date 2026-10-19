// Display-only title cleanup: emoji are stripped and mostly-uppercase Skool
// titles are turned into Spanish title case. Stored data, keys and search
// matching never use this.

// Emoji parts: keycaps (digit/#/* + optional VS16 + U+20E3), anything with
// default emoji presentation (incl. regional-indicator flags), text
// pictographs forced to emoji with VS16, skin-tone modifiers, ZWJ, variation
// selectors and tag characters (subdivision flags).
const EMOJI =
  /[0-9#*]\uFE0F?\u20E3|\p{Emoji_Presentation}|\p{Extended_Pictographic}\uFE0F|\p{Emoji_Modifier}|[\u200D\uFE0E\uFE0F]|[\u{E0020}-\u{E007F}]/gu;

/** Removes emoji and tidies spaces; returns the original if nothing else is left. */
export function stripEmoji(s: string): string {
  const out = s.replace(EMOJI, " ").replace(/\s+/g, " ").trim();
  return out || s;
}

const CONNECTORS = new Set([
  "de", "del", "la", "las", "el", "los", "lo", "y", "e", "o", "u", "ni", "en", "a", "al",
  "con", "para", "por", "un", "una", "que", "se", "su", "sus", "tu", "mi",
]);

/** Short uppercase words that are acronyms even though they contain vowels. */
const ACRONYMS = new Set(["IA", "AI", "SEO", "ROI", "CTA", "UX", "UI", "API", "KPI", "VIP", "TV", "USA", "ADN", "ONG", "FAQ", "PDF", "PDFS", "VSL", "VSLS", "CRM", "DAW", "DAWS"]);

const ROMAN = /^(?=[IVXLC])M{0,3}(C[MD]|D?C{0,3})(X[CL]|L?X{0,3})(I[XV]|V?I{0,3})$/;
const VOWELS = /[AEIOUÁÉÍÓÚÜ]/;

const letters = (s: string): string => s.replace(/[^\p{L}]/gu, "");
const isUpperWord = (w: string): boolean => {
  const l = letters(w);
  return l.length > 0 && l === l.toLocaleUpperCase("es") && l !== l.toLocaleLowerCase("es");
};

function keepAsIs(core: string): boolean {
  if (/\d/.test(core) || core.includes(".")) return true;
  if (!isUpperWord(core)) return true; // mixed case such as PDFs
  const l = letters(core);
  if (ROMAN.test(l)) return true;
  if (ACRONYMS.has(l)) return true;
  return l.length <= 4 && !VOWELS.test(l);
}

function caseWord(core: string, first: boolean): string {
  const lower = core.toLocaleLowerCase("es");
  if (!first && isUpperWord(core) && CONNECTORS.has(lower)) return lower;
  if (keepAsIs(core)) return core;
  const i = lower.search(/\p{L}/u);
  return i < 0 ? lower : lower.slice(0, i) + lower[i].toLocaleUpperCase("es") + lower.slice(i + 1);
}

export function displayTitle(raw: string): string {
  const title = stripEmoji(raw);
  const words = title.split(/(\s+)/);
  const lettered = words.filter((w) => letters(w).length > 0);
  if (!lettered.length) return title;
  const upper = lettered.filter(isUpperWord).length;
  if (upper / lettered.length < 0.7) return title;

  let first = true;
  return words
    .map((w) => {
      if (!letters(w).length) return w;
      const out = caseWord(w, first);
      first = /[:.?!¡]$/.test(w);
      return out;
    })
    .join("");
}
