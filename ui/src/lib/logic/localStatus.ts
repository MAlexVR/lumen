// Local download state of a Skool group, from the library scan. The folder
// name mirrors lumen-skool `group_folder_name` (strip emoji, sanitize).
import type { Group, Library, PlatformId } from "../types";
import { allGroups } from "./library";

export type LocalStatus = "none" | "partial" | "complete";

const ILLEGAL = /[<>:"/\\|?*\u0000-\u001f\u007f]/g;
const RESERVED = new Set(["CON", "PRN", "AUX", "NUL"]);

function isSymbol(cp: number): boolean {
  return (
    cp > 0xffff ||
    cp === 0x00a9 ||
    cp === 0x00ae ||
    cp === 0x2122 ||
    cp === 0x200d ||
    cp === 0x20e3 ||
    (cp >= 0x2190 && cp <= 0x21ff) ||
    (cp >= 0x2300 && cp <= 0x23ff) ||
    (cp >= 0x25a0 && cp <= 0x27bf) ||
    (cp >= 0x2b00 && cp <= 0x2bff) ||
    (cp >= 0xfe00 && cp <= 0xfe0f)
  );
}

export function groupFolderName(displayName: string): string {
  const stripped = [...displayName].filter((c) => !isSymbol(c.codePointAt(0) ?? 0)).join("").trim();
  let out = stripped.replace(ILLEGAL, "_").replace(/^ +/, "").replace(/[. ]+$/, "");
  if (RESERVED.has(out)) out = `_${out}`;
  out = out.slice(0, 80);
  return out || "_";
}

export const groupFolder = (displayName: string): string => `Skool/${groupFolderName(displayName)}`;

function statusOf(local: Group | undefined): LocalStatus {
  if (!local) return "none";
  if (local.status === "complete") return "complete";
  return local.status === "partial" || local.courses.length ? "partial" : "none";
}

/** Folder name match (case-insensitive) within one platform. */
export function localStatusByFolder(library: Library, platform: PlatformId, folder: string): LocalStatus {
  const want = folder.toLocaleLowerCase();
  return statusOf(allGroups(library).find((g) => g.platform === platform && g.name.toLocaleLowerCase() === want));
}

/** Skool community: its folder is derived from the display name. */
export const localStatus = (group: { name: string }, library: Library): LocalStatus =>
  localStatusByFolder(library, "skool", groupFolderName(group.name));
