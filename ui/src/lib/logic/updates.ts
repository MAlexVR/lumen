// "Actualizaciones" in Ajustes: the check state machine, error mapping,
// relative time, token field rules and a plain-text view of release notes.
// Pure; the component does the I/O.

export interface UpdateAsset {
  name: string;
  size: number;
  downloadUrl: string;
  kind: "windows-installer" | "rpm" | "deb" | "appimage";
}

export interface UpdateInfo {
  current: string;
  latest: string | null;
  isNewer: boolean;
  isPrerelease: boolean;
  name: string;
  publishedAt: string | null;
  notes: string;
  htmlUrl: string | null;
  asset: UpdateAsset | null;
  checkedAt: string;
}

const CODES = ["update_unauthorized", "rate_limited", "network", "no_releases", "invalid_response", "invalid_token", "internal"] as const;
export type UpdateErrorCode = (typeof CODES)[number];

export type UpdateState =
  | { step: "idle" }
  | { step: "checking" }
  | { step: "up_to_date"; info: UpdateInfo }
  | { step: "available"; info: UpdateInfo }
  | { step: "error"; code: UpdateErrorCode; resetAt: string | null; tokenSaved: boolean };

export type UpdateAction =
  | { type: "check" }
  | { type: "done"; info: UpdateInfo }
  | { type: "failed"; error: unknown; tokenSaved: boolean };

export const initialUpdate: UpdateState = { step: "idle" };

export function reduceUpdate(s: UpdateState, a: UpdateAction): UpdateState {
  switch (a.type) {
    case "check":
      return s.step === "checking" ? s : { step: "checking" };
    case "done":
      return a.info.isNewer && a.info.latest ? { step: "available", info: a.info } : { step: "up_to_date", info: a.info };
    case "failed": {
      const code = updateErrorCode(a.error);
      const message = typeof a.error === "object" && a.error ? (a.error as { message?: string }).message : undefined;
      return { step: "error", code, resetAt: code === "rate_limited" ? rateLimitReset(message) : null, tokenSaved: a.tokenSaved };
    }
  }
}

export function updateErrorCode(err: unknown): UpdateErrorCode {
  const code = typeof err === "object" && err ? (err as { code?: unknown }).code : undefined;
  return (CODES as readonly unknown[]).includes(code) ? (code as UpdateErrorCode) : "internal";
}

export type UpdateErrorKey =
  | "upd_err_private"
  | "upd_err_rejected"
  | `upd_err_${Exclude<UpdateErrorCode, "update_unauthorized">}`;

export function updateErrorKey(code: UpdateErrorCode, tokenSaved: boolean): UpdateErrorKey {
  if (code === "update_unauthorized") return tokenSaved ? "upd_err_rejected" : "upd_err_private";
  return `upd_err_${code}`;
}

export function rateLimitReset(message: string | undefined): string | null {
  return message?.match(/\d{4}-\d{2}-\d{2}T[\d:.]+Z/)?.[0] ?? null;
}

export function relativeTime(iso: string, now: number, locale: string): string {
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return "";
  const secs = Math.max(0, Math.round((now - then) / 1000));
  if (secs < 60) return locale === "es" ? "ahora mismo" : "just now";
  const rtf = new Intl.RelativeTimeFormat(locale, { numeric: "always", style: "short" });
  if (secs < 3600) return rtf.format(-Math.round(secs / 60), "minute");
  if (secs < 86400) return rtf.format(-Math.round(secs / 3600), "hour");
  return new Intl.RelativeTimeFormat(locale, { numeric: "always", style: "long" }).format(-Math.round(secs / 86400), "day");
}

const LAST_KEY = "lumen.updateLastChecked";
interface KV {
  getItem(k: string): string | null;
  setItem(k: string, v: string): void;
}

export function readLastChecked(store: KV): string | null {
  try {
    return store.getItem(LAST_KEY);
  } catch {
    return null;
  }
}

export function writeLastChecked(store: KV, iso: string): void {
  try {
    store.setItem(LAST_KEY, iso);
  } catch {
    // Storage blocked: the time is simply not remembered.
  }
}

/** What the token field holds, ready to send (the backend validates again). */
export function tokenDraft(raw: string): { value: string; valid: boolean } {
  const value = raw.trim();
  return { value, valid: value.length > 0 && value.length <= 255 && !/\s/.test(value) };
}

export type NoteBlock = { kind: "heading" | "item" | "text"; text: string };

const plain = (s: string) =>
  s
    .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/(\*\*|__)(.*?)\1/g, "$2")
    .replace(/`([^`]*)`/g, "$1")
    .trim();

/** Release notes as plain text blocks. Nothing is interpreted as HTML: the
 * view renders `text` as text. */
export function notesBlocks(markdown: string): NoteBlock[] {
  const out: NoteBlock[] = [];
  for (const raw of markdown.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line) continue;
    const heading = line.match(/^#{1,6}\s+(.*)$/);
    const item = line.match(/^[-*+]\s+(.*)$/);
    if (heading) out.push({ kind: "heading", text: plain(heading[1]) });
    else if (item) out.push({ kind: "item", text: plain(item[1]) });
    else out.push({ kind: "text", text: plain(line) });
  }
  return out;
}
