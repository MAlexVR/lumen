// Time display and resume-position rules for the player.

export function formatTime(seconds: number): string {
  const total = Number.isFinite(seconds) && seconds > 0 ? Math.floor(seconds) : 0;
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = String(total % 60).padStart(2, "0");
  return h ? `${h}:${String(m).padStart(2, "0")}:${s}` : `${m}:${s}`;
}

/** Margin at both ends of a video where a saved position is not worth resuming. */
export const RESUME_MARGIN = 5;
export const SAVE_INTERVAL_MS = 5000;

export function resumeSeek(saved: number, duration: number): number {
  if (!Number.isFinite(duration) || duration <= 0) return 0;
  return saved >= RESUME_MARGIN && saved < duration - RESUME_MARGIN ? saved : 0;
}

export const shouldSavePosition = (now: number, lastSave: number, force: boolean): boolean =>
  force || now - lastSave >= SAVE_INTERVAL_MS;

/** Formats a `YYYY-MM-DD` date (no time zone shift); null when invalid. */
export function formatIsoDate(iso: string | null, locale: string): string | null {
  const m = iso ? /^(\d{4})-(\d{2})-(\d{2})$/.exec(iso) : null;
  if (!m) return null;
  const [y, mo, d] = [Number(m[1]), Number(m[2]), Number(m[3])];
  const date = new Date(Date.UTC(y, mo - 1, d));
  if (date.getUTCFullYear() !== y || date.getUTCMonth() !== mo - 1 || date.getUTCDate() !== d) return null;
  return new Intl.DateTimeFormat(locale, { day: "numeric", month: "short", year: "numeric", timeZone: "UTC" })
    .format(date)
    .replace(/\./g, "")
    .replace(/ de /g, " ");
}
