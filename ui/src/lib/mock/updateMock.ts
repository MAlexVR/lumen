// Update commands for browser previews. `?update=available|current|unauthorized|
// ratelimited|network|norelease` picks the outcome of the check;
// `?token=saved` starts with a saved token.
import type { Api } from "../api";
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

export function createUpdateMock(params: URLSearchParams): Pick<
  Api,
  "updateCheck" | "updateSettings" | "updateSetIncludePrereleases" | "updateTokenStatus" | "updateTokenSave" | "updateTokenClear"
> {
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
    checkedAt: new Date().toISOString(),
  };
  const fail = (code: string, message: string) => Promise.reject({ code, message });
  return {
    updateCheck: async () => {
      switch (params.get("update")) {
        case "available":
          return {
            ...base,
            latest: "0.3.3",
            isNewer: true,
            name: "v0.3.3",
            publishedAt: "2026-10-17T18:30:00Z",
            notes: NOTES,
            htmlUrl: "https://github.com/MAlexVR/lumen/releases/tag/v0.3.3",
            asset: {
              name: "lumen-0.3.3-1.x86_64.rpm",
              size: 8_400_000,
              downloadUrl: "https://github.com/MAlexVR/lumen/releases/download/v0.3.3/lumen-0.3.3-1.x86_64.rpm",
              kind: "rpm",
            },
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
  };
}
