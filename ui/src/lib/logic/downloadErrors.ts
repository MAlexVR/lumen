// Normalizes whatever a skool_* command rejects with into a typed error.
import type { DownloadError, DownloadErrorCode } from "../types";

export const ERROR_CODES: readonly DownloadErrorCode[] = [
  "not_logged_in",
  "session_expired",
  "invalid_cookies",
  "invalid_credentials",
  "already_running",
  "no_library",
  "login_cancelled",
  "login_timeout",
  "network",
  "invalid_token",
  "invalid_url",
  "unsupported_url",
  "unavailable",
  "ytdlp_missing",
  "bot_challenge",
  "diagnose_running",
  "drm_protected",
  "internal",
];

const isCode = (v: unknown): v is DownloadErrorCode => typeof v === "string" && (ERROR_CODES as string[]).includes(v);

export function toDownloadError(err: unknown): DownloadError {
  if (typeof err === "string") {
    try {
      const parsed: unknown = JSON.parse(err);
      if (parsed && typeof parsed === "object" && "code" in parsed) return toDownloadError(parsed);
    } catch {
      /* plain message */
    }
    return { code: "internal", message: err };
  }
  if (err && typeof err === "object") {
    const o = err as { code?: unknown; message?: unknown };
    const message = typeof o.message === "string" ? o.message : "";
    return { code: isCode(o.code) ? o.code : "internal", message };
  }
  return { code: "internal", message: "" };
}
