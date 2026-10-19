import { describe, expect, it } from "vitest";
import { formatRoute, parseRoute, type Route } from "./route";

describe("parseRoute", () => {
  it("defaults to home", () => {
    expect(parseRoute("")).toEqual({ name: "home" });
    expect(parseRoute("#/")).toEqual({ name: "home" });
    expect(parseRoute("#/nowhere")).toEqual({ name: "home" });
  });

  it("parses simple screens", () => {
    expect(parseRoute("#/library")).toEqual({ name: "library", platform: null });
    expect(parseRoute("#/settings")).toEqual({ name: "settings" });
    expect(parseRoute("#/downloads")).toEqual({ name: "downloads", platform: null });
  });

  it("parses a library platform filter", () => {
    expect(parseRoute("#/library/hotmart")).toEqual({ name: "library", platform: "hotmart" });
  });

  it("keeps group ids with slashes in course and watch routes", () => {
    expect(parseRoute("#/course/hotmart%2FCurso%20-%20Vendedor/1")).toEqual({
      name: "course",
      group: "hotmart/Curso - Vendedor",
      course: 1,
    });
  });

  it("parses a download platform and accepts the Spanish alias", () => {
    expect(parseRoute("#/downloads/skool")).toEqual({ name: "downloads", platform: "skool" });
    expect(parseRoute("#/descargas")).toEqual({ name: "downloads", platform: null });
    expect(parseRoute("#/descargas/skool")).toEqual({ name: "downloads", platform: "skool" });
  });

  it("parses a course by group and index", () => {
    expect(parseRoute("#/course/G/3")).toEqual({ name: "course", group: "G", course: 3 });
  });

  it("rejects a non numeric course index", () => {
    expect(parseRoute("#/course/G/abc")).toEqual({ name: "home" });
  });

  it("parses a lesson key that contains slashes", () => {
    expect(parseRoute("#/watch/G/1-A%2F2-B%2F3-C%20%C3%A1")).toEqual({
      name: "watch",
      group: "G",
      key: "1-A/2-B/3-C á",
    });
  });

  it("survives malformed escapes", () => {
    expect(parseRoute("#/watch/G/%E0%A4%A")).toEqual({ name: "home" });
  });
});

describe("formatRoute", () => {
  it.each<Route>([
    { name: "home" },
    { name: "library", platform: null },
    { name: "library", platform: "skool" },
    { name: "course", group: "G #1", course: 12 },
    { name: "watch", group: "G", key: "1-A/2-B/3-C?x" },
    { name: "settings" },
    { name: "downloads", platform: null },
    { name: "downloads", platform: "udemy" },
  ])("round trips %j", (route) => {
    expect(parseRoute(formatRoute(route))).toEqual(route);
  });
});
