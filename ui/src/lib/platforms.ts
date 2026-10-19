// Download platforms Lumen offers. Logos are original artwork (resized
// copies, or the official SVG); each sits on a stage whose background
// matches the artwork's own edge so it never shows as a box on black. Brand
// colour appears only through the logos and their stages, never as UI accent.
import hotmart from "../assets/platforms/hotmart.webp";
import hotmartMark from "../assets/platforms/hotmart-mark.webp";
import pluralsight from "../assets/platforms/pluralsight.webp";
import skool from "../assets/platforms/skool.webp";
import udemy from "../assets/platforms/udemy.svg";
import youtube from "../assets/platforms/youtube.svg";
import type { Route } from "./logic/route";

export type PlatformStatus = "available" | "soon";

export interface Platform {
  id: string;
  name: string;
  logo: string;
  /** Edge colour of the logo artwork, used behind it when letterboxed. */
  stageBg: string;
  /** Padding inside the stage, in % of its width (bare wordmarks need air). */
  inset: number;
  status: PlatformStatus;
  route: Route;
  /** Small placements (tabs, headings): a crop of the original artwork that
   * keeps the wordmark legible, on its own background. Defaults to the logo. */
  mark: { src: string; bg: string; /** Optical size correction inside the shared chip (1 = fit). */ scale: number };
}

interface Options {
  status?: PlatformStatus;
  inset?: number;
  mark?: { src: string; bg: string };
  markScale?: number;
}

const p = (id: string, name: string, logo: string, stageBg: string, o: Options = {}): Platform => ({
  id,
  name,
  logo,
  stageBg,
  inset: o.inset ?? 0,
  status: o.status ?? "soon",
  route: { name: "downloads", platform: id },
  mark: { ...(o.mark ?? { src: logo, bg: stageBg }), scale: o.markScale ?? 1 },
});

export const PLATFORMS: readonly Platform[] = [
  p("hotmart", "Hotmart", hotmart, "#f65124", { status: "available", mark: { src: hotmartMark, bg: "#ffffff" }, markScale: 1.3 }),
  p("skool", "Skool", skool, "#ffffff", { status: "available", markScale: 1.85 }),
  p("pluralsight", "Pluralsight", pluralsight, "#ffffff", { status: "available", markScale: 3.3 }),
  p("youtube", "YouTube", youtube, "#ffffff", { status: "available", inset: 27.5, markScale: 0.92 }),
  p("udemy", "Udemy", udemy, "#ffffff", { status: "available", inset: 24, markScale: 1.05 }),
];

export const platformById = (id: string | null): Platform | undefined =>
  id ? PLATFORMS.find((x) => x.id === id) : undefined;

/** Roving-focus target for arrow/Home/End keys in a grid of `count` tiles. */
export function gridMove(index: number, key: string, cols: number, count: number): number | null {
  const last = count - 1;
  switch (key) {
    case "ArrowRight":
      return Math.min(last, index + 1);
    case "ArrowLeft":
      return Math.max(0, index - 1);
    case "ArrowDown":
      return Math.min(last, index + cols);
    case "ArrowUp":
      return index - cols >= 0 ? index - cols : index;
    case "Home":
      return 0;
    case "End":
      return last;
    default:
      return null;
  }
}
