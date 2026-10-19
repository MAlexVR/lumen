import { describe, expect, it } from "vitest";
import {
  initialInstall,
  installAction,
  installErrorCode,
  installErrorKey,
  installKindKeys,
  progressLabel,
  progressPercent,
  reduceInstall,
} from "./updateInstall";

describe("reduceInstall", () => {
  it("asks, confirms, downloads, verifies, installs and restarts", () => {
    let s = reduceInstall(initialInstall, { type: "ask" });
    expect(s.step).toBe("confirming");
    s = reduceInstall(s, { type: "start" });
    expect(s).toMatchObject({ step: "running", stage: "downloading", downloaded: 0, total: 0 });
    s = reduceInstall(s, { type: "progress", stage: "downloading", downloaded: 10, total: 40 });
    expect(s).toMatchObject({ stage: "downloading", downloaded: 10, total: 40 });
    for (const stage of ["verifying", "installing", "restarting"] as const) {
      s = reduceInstall(s, { type: "progress", stage, downloaded: 40, total: 40 });
      expect(s).toMatchObject({ step: "running", stage });
    }
  });

  it("cancel from confirming goes back to idle; start only works from confirming", () => {
    const c = reduceInstall(initialInstall, { type: "ask" });
    expect(reduceInstall(c, { type: "dismiss" })).toEqual(initialInstall);
    expect(reduceInstall(initialInstall, { type: "start" })).toBe(initialInstall);
    const run = reduceInstall(c, { type: "start" });
    expect(reduceInstall(run, { type: "ask" })).toBe(run);
  });

  it("ignores progress when nothing is running and never goes back a stage", () => {
    expect(reduceInstall(initialInstall, { type: "progress", stage: "verifying", downloaded: 1, total: 1 })).toBe(initialInstall);
    let s = reduceInstall(reduceInstall(initialInstall, { type: "ask" }), { type: "start" });
    s = reduceInstall(s, { type: "progress", stage: "installing", downloaded: 40, total: 40 });
    expect(reduceInstall(s, { type: "progress", stage: "downloading", downloaded: 5, total: 40 })).toBe(s);
  });

  it("fails with a code, the message and the file path, and can be dismissed", () => {
    const run = reduceInstall(reduceInstall(initialInstall, { type: "ask" }), { type: "start" });
    const e = reduceInstall(run, { type: "failed", error: { code: "manual_install_required", message: "m", path: "/tmp/L.AppImage" } });
    expect(e).toEqual({ step: "error", code: "manual_install_required", message: "m", path: "/tmp/L.AppImage" });
    expect(reduceInstall(e, { type: "dismiss" })).toEqual(initialInstall);
  });
});

describe("installErrorCode / Key", () => {
  it("reads known codes and falls back to internal", () => {
    expect(installErrorCode({ code: "signature_invalid" })).toBe("signature_invalid");
    expect(installErrorCode({ code: "nope" })).toBe("internal");
    expect(installErrorCode("x")).toBe("internal");
  });
  it("has a message key for every code", () => {
    expect(installErrorKey("checksum_mismatch")).toBe("upd_ierr_checksum_mismatch");
    expect(installErrorKey("up_to_date")).toBe("upd_ierr_up_to_date");
  });
});

describe("progress formatting", () => {
  it("shows MB and a rounded percent, clamped", () => {
    expect(progressPercent(10, 40)).toBe(25);
    expect(progressPercent(50, 40)).toBe(100);
    expect(progressPercent(5, 0)).toBeNull();
    expect(progressLabel(12_300_000, 40_500_000, "es")).toBe("12,3 / 40,5 MB · 30 %");
    expect(progressLabel(12_300_000, 40_500_000, "en")).toBe("12.3 / 40.5 MB · 30%");
    expect(progressLabel(0, 0, "es")).toBe("");
  });
});

describe("installKindKeys", () => {
  it("names the kind and what will happen", () => {
    expect(installKindKeys("rpm")).toEqual({ kind: "upd_kind_rpm", note: "upd_note_rpm" });
    expect(installKindKeys("appimage").note).toBe("upd_note_appimage");
    expect(installKindKeys("windows-nsis").kind).toBe("upd_kind_windows");
    expect(installKindKeys("unmanaged").kind).toBe("upd_kind_manual");
  });
});

describe("installAction", () => {
  const base = { installKind: "rpm" as const, signed: true, manualInstallRequired: false, hasAsset: true };
  it("is enabled only when signed, with an asset and nothing running", () => {
    expect(installAction(base, initialInstall)).toEqual({ label: "install", enabled: true, reason: null });
    expect(installAction({ ...base, signed: false }, initialInstall)).toEqual({ label: "install", enabled: false, reason: "unsigned" });
    expect(installAction(base, { step: "running", stage: "downloading", downloaded: 0, total: 0 }).enabled).toBe(false);
    expect(installAction(base, { step: "confirming" }).enabled).toBe(false);
    expect(installAction({ ...base, hasAsset: false }, initialInstall).enabled).toBe(false);
  });
  it("downloads and verifies only for a manual copy", () => {
    const manual = installAction({ ...base, installKind: "unmanaged", manualInstallRequired: true, hasAsset: false }, initialInstall);
    expect(manual).toEqual({ label: "download", enabled: true, reason: null });
  });
});
