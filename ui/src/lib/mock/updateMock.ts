// Update commands for browser previews. `?update=available|current|unauthorized|
// ratelimited|network|norelease` picks the outcome of the check;
// `?token=saved` starts with a saved token.
import type { Api } from "../api";
import type { InstallStage } from "../logic/updateInstall";
import type { UpdateInfo } from "../logic/updates";

const NOTES = [
  "## What's new",
  "",
  "- **Thumbnails** on every card, generated once and cached",
  "- Course poster plays a short preview on hover",
  "- Faster Biblioteca with hundreds of lessons",
  "",
  "Fixes: the library refreshes after a download finishes.",
].join("\n");

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const TOTAL = 40_500_000;

export function createUpdateMock(params: URLSearchParams): Pick<
  Api,
  | "updateCheck"
  | "updateSettings"
  | "updateSetIncludePrereleases"
  | "updateTokenStatus"
  | "updateTokenSave"
  | "updateTokenClear"
  | "updateInstall"
  | "updateCancel"
  | "onUpdateProgress"
> {
  const listeners = new Set<(e: { stage: InstallStage; downloaded: number; total: number }) => void>();
  const emit = (stage: InstallStage, downloaded: number) => listeners.forEach((cb) => cb({ stage, downloaded, total: TOTAL }));
  let cancelled = false;
  // `?hold=downloading` stops at 42 %, `?hold=installing` stops while installing (for screenshots).
  const hold = params.get("hold");
  let saved = params.get("token") === "saved";
  let includePrereleases = true;
  const base: UpdateInfo = {
    current: "0.3.2",
    latest: "0.3.2",
    isNewer: false,
    isPrerelease: true,
    name: "v0.3.2",
    publishedAt: "2026-10-10T09:00:00Z",
    notes: "",
    htmlUrl: "https://github.com/MAlexVR/lumen/releases/tag/v0.3.2",
    asset: null,
    installKind: "rpm",
    manualInstallRequired: false,
    signed: true,
    checkedAt: new Date().toISOString(),
  };
  const fail = (code: string, message: string) => Promise.reject({ code, message });
  return {
    updateCheck: async () => {
      switch (params.get("update")) {
        case "available":
        case "install-ok":
        case "install-cancel":
        case "install-sig":
        case "install-manual":
        case "install-fail":
        case "unsigned":
          return {
            ...base,
            latest: "0.3.3",
            isNewer: true,
            name: "v0.3.3",
            publishedAt: "2026-10-17T18:30:00Z",
            notes: NOTES,
            htmlUrl: "https://github.com/MAlexVR/lumen/releases/tag/v0.3.3",
            asset: params.get("update") === "install-manual" ? null : {
              name: "lumen-0.3.3-1.x86_64.rpm",
              size: 8_400_000,
              downloadUrl: "https://github.com/MAlexVR/lumen/releases/download/v0.3.3/lumen-0.3.3-1.x86_64.rpm",
              kind: "rpm",
            },
            installKind: params.get("update") === "install-manual" ? "unmanaged" : "rpm",
            manualInstallRequired: params.get("update") === "install-manual",
            signed: params.get("update") !== "unsigned",
            checkedAt: new Date().toISOString(),
          };
        case "unauthorized":
          return fail("update_unauthorized", saved ? "the saved token was rejected or lacks access" : "the repository is private");
        case "ratelimited":
          return fail("rate_limited", `rate limit exceeded, resets at ${new Date(Date.now() + 25 * 60_000).toISOString()}`);
        case "network":
          return fail("network", "cannot connect");
        case "norelease":
          return fail("no_releases", "no releases");
        default:
          return { ...base, checkedAt: new Date().toISOString() };
      }
    },
    updateSettings: async () => ({ includePrereleases }),
    updateSetIncludePrereleases: async (v) => void (includePrereleases = v),
    updateTokenStatus: async () => ({ saved }),
    updateTokenSave: async () => void (saved = true),
    updateTokenClear: async () => void (saved = false),
    onUpdateProgress: async (cb) => {
      listeners.add(cb);
      return () => void listeners.delete(cb);
    },
    updateCancel: async () => {
      cancelled = true;
      return true;
    },
    updateInstall: async () => {
      const mode = params.get("update");
      cancelled = false;
      for (let i = 0; i <= 10; i++) {
        if (cancelled) return fail("cancelled", "cancelled");
        emit("downloading", Math.round((TOTAL * (hold === "downloading" ? 0.42 : i / 10))));
        if (hold === "downloading") return new Promise<never>(() => {});
        await sleep(250);
      }
      emit("verifying", TOTAL);
      await sleep(600);
      if (mode === "install-sig") return fail("signature_invalid", "bad signature");
      if (mode === "install-manual") return Promise.reject({ code: "manual_install_required", message: "verified", path: "/home/usuario/.cache/lumen/updates/0.3.3/Lumen_0.3.3_amd64.AppImage" });
      emit("installing", TOTAL);
      if (hold === "installing") return new Promise<never>(() => {});
      await sleep(900);
      if (mode === "install-cancel") return fail("install_cancelled", "authorization dismissed");
      if (mode === "install-fail") return fail("install_failed", "Error: package conflicts with lumen-0.3.2");
      emit("restarting", TOTAL);
      return new Promise<never>(() => {});
    },
  };
}
