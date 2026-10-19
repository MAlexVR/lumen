// Installing an update from Ajustes: the confirm → download → verify →
// install → restart state machine, error mapping, progress text, install-kind
// copy and the rule for when the button may be pressed. Pure.

export type InstallKind = "windows-nsis" | "appimage" | "rpm" | "deb" | "unmanaged";
export type InstallStage = "downloading" | "verifying" | "installing" | "restarting";

const STAGES: readonly InstallStage[] = ["downloading", "verifying", "installing", "restarting"];

const CODES = [
  "update_unauthorized",
  "rate_limited",
  "network",
  "no_releases",
  "invalid_response",
  "checksum_mismatch",
  "signature_missing",
  "signature_invalid",
  "install_cancelled",
  "install_failed",
  "manual_install_required",
  "already_running",
  "cancelled",
  "disk",
  "up_to_date",
  "internal",
] as const;
export type InstallErrorCode = (typeof CODES)[number];

export type InstallState =
  | { step: "idle" }
  | { step: "confirming" }
  | { step: "running"; stage: InstallStage; downloaded: number; total: number }
  | { step: "error"; code: InstallErrorCode; message: string; path: string | null };

export type InstallAction =
  | { type: "ask" }
  | { type: "dismiss" }
  | { type: "start" }
  | { type: "progress"; stage: InstallStage; downloaded: number; total: number }
  | { type: "failed"; error: unknown };

export const initialInstall: InstallState = { step: "idle" };

export function reduceInstall(s: InstallState, a: InstallAction): InstallState {
  switch (a.type) {
    case "ask":
      return s.step === "idle" || s.step === "error" ? { step: "confirming" } : s;
    case "dismiss":
      return s.step === "confirming" || s.step === "error" ? initialInstall : s;
    case "start":
      return s.step === "confirming" ? { step: "running", stage: "downloading", downloaded: 0, total: 0 } : s;
    case "progress":
      if (s.step !== "running" || STAGES.indexOf(a.stage) < STAGES.indexOf(s.stage)) return s;
      return { step: "running", stage: a.stage, downloaded: a.downloaded, total: a.total };
    case "failed": {
      const o = typeof a.error === "object" && a.error ? (a.error as { message?: unknown; path?: unknown }) : {};
      return {
        step: "error",
        code: installErrorCode(a.error),
        message: typeof o.message === "string" ? o.message : "",
        path: typeof o.path === "string" ? o.path : null,
      };
    }
  }
}

export function installErrorCode(err: unknown): InstallErrorCode {
  const code = typeof err === "object" && err ? (err as { code?: unknown }).code : undefined;
  return (CODES as readonly unknown[]).includes(code) ? (code as InstallErrorCode) : "internal";
}

export const installErrorKey = (code: InstallErrorCode) => `upd_ierr_${code}` as const;

export function progressPercent(downloaded: number, total: number): number | null {
  if (!(total > 0)) return null;
  return Math.min(100, Math.max(0, Math.round((downloaded / total) * 100)));
}

export function progressLabel(downloaded: number, total: number, locale: string): string {
  const pct = progressPercent(downloaded, total);
  if (pct === null) return "";
  const n = new Intl.NumberFormat(locale, { minimumFractionDigits: 1, maximumFractionDigits: 1 });
  return `${n.format(downloaded / 1e6)} / ${n.format(total / 1e6)} MB · ${pct}${locale === "es" ? " " : ""}%`;
}

export function installKindKeys(kind: InstallKind) {
  const name = kind === "windows-nsis" ? "windows" : kind === "unmanaged" ? "manual" : kind;
  return { kind: `upd_kind_${name}`, note: `upd_note_${kind === "windows-nsis" ? "windows" : kind}` } as const;
}

export interface InstallFacts {
  installKind: InstallKind;
  signed: boolean;
  manualInstallRequired: boolean;
  hasAsset: boolean;
}

/** The primary button: what it does and whether it may be pressed. */
export function installAction(f: InstallFacts, s: InstallState) {
  const label = f.installKind === "unmanaged" || f.manualInstallRequired ? ("download" as const) : ("install" as const);
  if (!f.signed) return { label, enabled: false, reason: "unsigned" as const };
  if (!f.hasAsset && f.installKind !== "unmanaged") return { label, enabled: false, reason: "no_asset" as const };
  if (s.step === "running" || s.step === "confirming") return { label, enabled: false, reason: "busy" as const };
  return { label, enabled: true, reason: null };
}
