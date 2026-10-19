import { describe, expect, it } from "vitest";
import { initialPicker, middleTruncate, reducePicker, rootErrorKind, summaryMessage } from "./libraryRoot";

describe("rootErrorKind", () => {
  it.each([
    ["choose a folder", "empty"],
    ["the folder path must be absolute", "not_absolute"],
    ["that folder cannot be used as a library: the filesystem root", "forbidden_root"],
    ["that folder cannot be used as a library: a system folder", "forbidden_system"],
    ["that folder cannot be used as a library: your home folder itself, pick a subfolder", "forbidden_home"],
    ["that folder cannot be used as a library: a folder that contains your home folder", "forbidden_home"],
    ["that path is a file, not a folder", "not_directory"],
    ["the folder is not writable", "not_writable"],
    ["cannot use that folder: Permission denied (os error 13)", "io"],
    ["something unexpected", "unknown"],
  ])("maps %j", (message, kind) => {
    expect(rootErrorKind(message)).toBe(kind);
  });

  it("accepts Error objects and non-strings", () => {
    expect(rootErrorKind(new Error("the folder is not writable"))).toBe("not_writable");
    expect(rootErrorKind(undefined)).toBe("unknown");
  });
});

describe("summaryMessage", () => {
  it("lists the platforms already in the folder", () => {
    expect(summaryMessage({ platforms: ["Skool", "Hotmart"], hasLibrary: true })).toEqual({
      key: "lr_summary_has",
      platforms: "Skool, Hotmart",
    });
  });

  it("explains an empty folder", () => {
    expect(summaryMessage({ platforms: [], hasLibrary: false })).toEqual({ key: "lr_summary_empty", platforms: "" });
  });
});

describe("reducePicker", () => {
  it("goes from picking to reviewing with a summary, then saving and back to idle", () => {
    let s = reducePicker(initialPicker, { type: "pick" });
    expect(s).toEqual({ step: "picking" });
    s = reducePicker(s, { type: "picked", path: "/home/u/Cursos" });
    expect(s).toEqual({ step: "reviewing", path: "/home/u/Cursos", summary: null });
    s = reducePicker(s, { type: "summary", summary: { platforms: ["Skool"], hasLibrary: true } });
    expect(s).toMatchObject({ step: "reviewing", summary: { platforms: ["Skool"] } });
    s = reducePicker(s, { type: "confirm" });
    expect(s).toEqual({ step: "saving", path: "/home/u/Cursos" });
    expect(reducePicker(s, { type: "saved" })).toEqual(initialPicker);
  });

  it("treats a cancelled native dialog as nothing happened", () => {
    expect(reducePicker({ step: "picking" }, { type: "picked", path: null })).toEqual(initialPicker);
  });

  it("keeps the chosen path when saving fails so the user can see why", () => {
    const s = reducePicker({ step: "saving", path: "/x" }, { type: "failed", kind: "not_writable" });
    expect(s).toEqual({ step: "error", path: "/x", kind: "not_writable" });
  });

  it("ignores a summary that arrives for another step", () => {
    expect(reducePicker(initialPicker, { type: "summary", summary: { platforms: [], hasLibrary: false } })).toEqual(initialPicker);
  });

  it("cancels from any step", () => {
    expect(reducePicker({ step: "reviewing", path: "/x", summary: null }, { type: "cancel" })).toEqual(initialPicker);
    expect(reducePicker({ step: "error", path: "/x", kind: "io" }, { type: "cancel" })).toEqual(initialPicker);
  });
});

describe("middleTruncate", () => {
  it("keeps short paths", () => {
    expect(middleTruncate("/home/u/Cursos", 40)).toBe("/home/u/Cursos");
  });

  it("keeps the start and the end (the folder name) of long paths", () => {
    const p = "/home/usuario/Vídeos/Cursos descargados/Biblioteca principal";
    const out = middleTruncate(p, 32);
    expect(out.length).toBe(32);
    expect(out.startsWith("/home/usuar")).toBe(true);
    expect(out.endsWith("Biblioteca principal")).toBe(true);
    expect(out).toContain("…");
  });
});
