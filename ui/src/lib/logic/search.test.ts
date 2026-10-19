import { describe, expect, it } from "vitest";
import type { Course } from "../types";
import { filterCourses, normalize } from "./search";

const c = (title: string, lessons: string[] = []): Course => ({
  title,
  index: 1,
  sections: [
    {
      title: "S",
      index: 1,
      lessons: lessons.map((t, i) => ({
        key: t,
        title: t,
        index: i + 1,
        video: null,
        description: null,
        hasVideo: false,
        resources: [],
      })),
    },
  ],
});

describe("normalize", () => {
  it("lowercases and strips accents and extra spaces", () => {
    expect(normalize("  Edición   RÁPIDA ")).toBe("edicion rapida");
  });
});

describe("filterCourses", () => {
  const courses = [c("La mesa de edición"), c("Modo Cabaña", ["Rutina de enfoque"]), c("Guía Rápida")];

  it("returns every course for an empty query", () => {
    expect(filterCourses(courses, "   ")).toHaveLength(3);
  });

  it("matches course titles ignoring case and accents", () => {
    expect(filterCourses(courses, "EDICION").map((x) => x.title)).toEqual(["La mesa de edición"]);
  });

  it("matches lesson titles", () => {
    expect(filterCourses(courses, "enfoque").map((x) => x.title)).toEqual(["Modo Cabaña"]);
  });

  it("requires every word to match", () => {
    expect(filterCourses(courses, "guía cabaña")).toHaveLength(0);
    expect(filterCourses(courses, "modo rutina").map((x) => x.title)).toEqual(["Modo Cabaña"]);
  });
});
