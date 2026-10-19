// Platform-aware views of the scanned library (Skool communities and
// Hotmart purchased courses share the Group > Course > Section > Lesson tree).
import type { Course, Group, GroupProgress, Library, PlatformId } from "../types";
import { continueTarget, flattenGroup, type ContinueTarget } from "./progress";
import { normalize } from "./search";
import { stripEmoji } from "./title";

const PLATFORM_ORDER: readonly PlatformId[] = ["skool", "hotmart", "pluralsight", "youtube"];

/** Platforms whose groups are a single course (opened straight into it). */
const SINGLE_COURSE: readonly PlatformId[] = ["hotmart", "pluralsight", "youtube"];

export function allGroups(library: Library): Group[] {
  if (!Array.isArray(library.platforms)) return library.groups ?? [];
  return [...library.platforms]
    .sort((a, b) => PLATFORM_ORDER.indexOf(a.id) - PLATFORM_ORDER.indexOf(b.id))
    .flatMap((p) => p.groups);
}

export const groupById = (library: Library, id: string): Group | undefined =>
  allGroups(library).find((g) => g.id === id);

export const platformsPresent = (library: Library): PlatformId[] =>
  PLATFORM_ORDER.filter((id) => library.platforms?.some((p) => p.id === id && p.groups.length));

/** Hotmart purchases and YouTube playlists are one course: skip the group level. */
export const opensStraightIntoCourse = (group: Group): boolean =>
  SINGLE_COURSE.includes(group.platform) && group.courses.length === 1;

export interface LibraryEntry {
  group: Group;
  courses: Course[];
}

/** Groups (optionally of one platform) with the courses matching `query`
 * on group title/seller, course, module or lesson titles. */
export function libraryEntries(library: Library, platform: string | null, query: string): LibraryEntry[] {
  const words = normalize(query).split(" ").filter(Boolean);
  return allGroups(library)
    .filter((g) => !platform || g.platform === platform)
    .map((group) => {
      const head = [group.title, group.seller ?? ""].join(" ");
      const courses = group.courses.filter((c) => {
        if (!words.length) return true;
        const text = [head, c.title, ...c.sections.flatMap((s) => [s.title, ...s.lessons.map((l) => l.title)])].join(" ");
        // Match on the original text and on what is displayed (emoji stripped).
        const hay = normalize(`${text} ${stripEmoji(text)}`);
        return words.every((w) => hay.includes(w));
      });
      return { group, courses };
    })
    .filter((e) => e.courses.length);
}

export interface Shelf {
  key: string;
  platform: PlatformId;
  /** Group title for Skool; empty for the Hotmart shelf (the caller labels it). */
  title: string;
  items: { group: Group; course: Course }[];
}

/** Home shelves: one per Skool community, one per single-course platform. */
export function shelves(library: Library): Shelf[] {
  const out: Shelf[] = [];
  const groups = allGroups(library);
  for (const g of groups.filter((x) => x.platform === "skool"))
    out.push({ key: g.id, platform: "skool", title: g.title, items: g.courses.map((course) => ({ group: g, course })) });
  for (const platform of SINGLE_COURSE) {
    const mine = groups.filter((x) => x.platform === platform);
    if (mine.length)
      out.push({ key: platform, platform, title: "", items: mine.flatMap((g) => g.courses.map((course) => ({ group: g, course }))) });
  }
  return out;
}

/** "Continue where you left off" across platforms. The backend keeps no
 * timestamps, so the most recent group is the one last opened in this app
 * (`recentId`); without history in it, the first group with history wins. */
export function continueAcross(
  library: Library,
  progressOf: (groupId: string) => GroupProgress,
  recentId: string | null,
): { group: Group; target: ContinueTarget } | null {
  const groups = allGroups(library);
  const candidates = [
    groups.find((g) => g.id === recentId && progressOf(g.id).lastLesson),
    groups.find((g) => progressOf(g.id).lastLesson),
  ];
  for (const g of candidates) {
    const target = g ? continueTarget(flattenGroup(g), progressOf(g.id)) : null;
    if (g && target?.kind === "resume") return { group: g, target };
  }
  for (const g of groups) {
    const target = continueTarget(flattenGroup(g), progressOf(g.id));
    if (target) return { group: g, target };
  }
  return null;
}
