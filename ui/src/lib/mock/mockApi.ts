// In-memory stand-in for the Tauri commands, built from a fixture generated
// from a real library (scripts/ui-fixture.py). Used for browser previews and
// headless screenshots; nothing here is persisted.
import { mediaUrl, type Api } from "../api";
import { allGroups } from "../logic/library";
import type { GroupProgress, Library } from "../types";
import fixture from "./library.json";
import { createBus, createGoldenlifeMock, createHarbizMock, createHotmartMock, createPluralsightMock, createSkoolMock } from "./downloadsMock";
import { createLibraryMock } from "./libraryMock";
import { createYoutubeMock, youtubeLibrary } from "./youtubeMock";
import { createUpdateMock } from "./updateMock";
import { createUdemyMock } from "./udemyMock";

const SAMPLE_DESCRIPTION = `<!doctype html><html><head><style>body{background:#fff;color:#222;font-family:Arial}</style></head><body>
<h2>Sobre esta clase</h2>
<p>En esta lección repasamos los conceptos clave del módulo y los aplicamos a un caso real. Toma notas de las preguntas que te surjan.</p>
<ul><li>Objetivo de la sesión</li><li>Ejemplo práctico paso a paso</li><li>Tarea para la próxima clase</li></ul>
<p>Material complementario: <a href="https://example.com/guia">guía en PDF</a>.</p>
</body></html>`;

/** Demo history so the preview shows every progress state. */
function seedProgress(library: Library): Map<string, GroupProgress> {
  const out = new Map<string, GroupProgress>();
  for (const group of allGroups(library)) {
    const lessons = group.courses.flatMap((c) => c.sections.flatMap((s) => s.lessons));
    const watched: string[] = [];
    group.courses.forEach((c, ci) => {
      const keys = c.sections.flatMap((s) => s.lessons.map((l) => l.key));
      const share =
        group.platform === "hotmart"
          ? [0.4, 0.15, 0, 1][allGroups(library).filter((g) => g.platform === "hotmart").indexOf(group)] ?? 0
          : ci === 0 ? 1 : ci === 1 ? 0.6 : ci === 4 ? 0.12 : ci === 2 ? 0.3 : 0;
      watched.push(...keys.slice(0, Math.round(keys.length * share)));
    });
    const last = lessons.find((l) => !watched.includes(l.key) && l.hasVideo) ?? null;
    out.set(group.id, {
      watched,
      lastLesson: last?.key ?? null,
      positions: last ? [{ lessonKey: last.key, seconds: 95 }] : [],
    });
  }
  return out;
}

export function createMockApi(): Api {
  // Live copy: mock deletions remove entries from it.
  const library = structuredClone(fixture) as Library;
  library.platforms.push(youtubeLibrary());
  library.platforms.push({
    id: "pluralsight",
    groups: [{
      id: "pluralsight/JavaScript Fundamentals-0123456789abcdef",
      platform: "pluralsight",
      name: "JavaScript Fundamentals-0123456789abcdef",
      title: "JavaScript Fundamentals",
      seller: null,
      status: "partial",
      lessonCount: 2,
      courses: [{
        title: "JavaScript Fundamentals",
        index: 1,
        sections: [{
          title: "Getting Started",
          index: 1,
          lessons: [
            { key: "pluralsight:JavaScript Fundamentals/Getting Started/Variables", title: "Variables and Values", index: 1, video: "Pluralsight/JavaScript Fundamentals-0123456789abcdef/01 - Getting Started/01 - Variables and Values-0123456789abcdef/video.mp4", description: null, hasVideo: true, resources: [] },
            { key: "pluralsight:JavaScript Fundamentals/Getting Started/Functions", title: "Functions and Scope", index: 2, video: "Pluralsight/JavaScript Fundamentals-0123456789abcdef/01 - Getting Started/02 - Functions and Scope-0123456789abcdef/video.mp4", description: "Pluralsight/JavaScript Fundamentals-0123456789abcdef/01 - Getting Started/02 - Functions and Scope-0123456789abcdef/description.html", hasVideo: true, resources: [] },
          ],
        }],
      }],
    }],
  });
  library.platforms.push({
    id: "goldenlife",
    groups: [{
      id: "goldenlife/Creator Basics",
      platform: "goldenlife",
      name: "Creator Basics",
      title: "Creator Basics",
      seller: null,
      status: "partial",
      lessonCount: 2,
      courses: [{
        title: "Creator Basics",
        index: 1,
        sections: [{
          title: "Welcome",
          index: 1,
          lessons: [
            { key: "goldenlife:Creator Basics/1. Welcome/1. Start here", title: "Start here", index: 1, video: "GoldenLife/Creator Basics/1. Welcome/1. Start here/1. Start here.mp4", description: null, hasVideo: true, resources: [] },
            { key: "goldenlife:Creator Basics/1. Welcome/2. Your first video", title: "Your first video", index: 2, video: "GoldenLife/Creator Basics/1. Welcome/2. Your first video/2. Your first video.mp4", description: "GoldenLife/Creator Basics/1. Welcome/2. Your first video/Description.html", hasVideo: true, resources: [] },
          ],
        }],
      }],
    }],
  });
  library.platforms.push({
    id: "harbiz",
    groups: [{
      id: "harbiz/Harbiz Training",
      platform: "harbiz",
      name: "Harbiz Training",
      title: "Harbiz Training",
      seller: null,
      status: "partial",
      lessonCount: 2,
      courses: [{
        title: "Harbiz Training",
        index: 1,
        sections: [{
          title: "2026-06-02 - Hip mobility",
          index: 0,
          lessons: [
            { key: "harbiz:1-Harbiz Training/0-2026-06-02 - Hip mobility/1-Bodyweight squat", title: "Bodyweight squat", index: 1, video: "Harbiz/Harbiz Training/2026-06-02 - Hip mobility/01. Bodyweight squat.mp4", description: "Harbiz/Harbiz Training/2026-06-02 - Hip mobility/01. Bodyweight squat.html", hasVideo: true, resources: [] },
            { key: "harbiz:1-Harbiz Training/0-2026-06-02 - Hip mobility/2-Leg swings", title: "Leg swings", index: 2, video: "Harbiz/Harbiz Training/2026-06-02 - Hip mobility/02. Leg swings.mp4", description: null, hasVideo: true, resources: [] },
          ],
        }],
      }],
    }],
  });
  const progress = seedProgress(library);
  const params = new URLSearchParams(location.search);
  const bus = createBus();
  const get = (group: string): GroupProgress => {
    let p = progress.get(group);
    if (!p) {
      p = { watched: [], lastLesson: null, positions: [] };
      progress.set(group, p);
    }
    return p;
  };
  return {
    mode: "mock",
    appInfo: async () => ({ name: "Lumen", version: "preview" }),
    // `?media=http://127.0.0.1:8765` points the preview at a local file server.
    mediaBase: async () => params.get("media") ?? "",
    defaultRoot: async () => "/home/usuario/Lumen",
    scanLibrary: async () => structuredClone(library),
    progress: async (group) => structuredClone(get(group)),
    setWatched: async (group, key, watched) => {
      const p = get(group);
      p.watched = p.watched.filter((k) => k !== key);
      if (watched) p.watched.push(key);
    },
    setLast: async (group, key) => {
      get(group).lastLesson = key;
    },
    savePosition: async (group, key, seconds) => {
      const p = get(group);
      p.positions = [...p.positions.filter((x) => x.lessonKey !== key), { lessonKey: key, seconds }];
    },
    // With a local media server the real description file is shown.
    description: async (path) => {
      const media = params.get("media");
      if (!media) return SAMPLE_DESCRIPTION;
      const res = await fetch(mediaUrl(media, path));
      return res.ok ? res.text() : SAMPLE_DESCRIPTION;
    },
    openPath: async (path) => console.info("[mock] open", path),
    openFolder: async (rel) => console.info("[mock] open folder", rel ?? "(root)"),
    openUrl: async (url) => console.info("[mock] open", url),
    ...createLibraryMock(params, library),
    ...createYoutubeMock(params.get("youtube"), bus),
    // No ffmpeg in previews: every thumbnail is unavailable, cards keep the poster.
    ...createUpdateMock(params),
    thumbnailsRequest: async (items) => items.map((i) => ({ ...i, state: "unavailable" as const, reason: "ffmpeg_missing" })),
    thumbnailsCancel: async () => {},
    thumbnailsStatus: async () => ({ ffmpegAvailable: params.get("ffmpeg") !== "missing", cacheBytes: 0, queued: 0, running: 0 }),
    onThumbnailReady: async () => () => {},
    ...createUdemyMock(params.get("udemy")),
    ...createSkoolMock(params.get("skool"), bus),
    ...createHotmartMock(params.get("hotmart"), bus),
    ...createPluralsightMock(params.get("pluralsight"), bus),
    ...createGoldenlifeMock(params.get("goldenlife"), bus),
    ...createHarbizMock(params.get("harbiz"), bus),
    downloadPause: async (id) => bus.setPaused(id, true),
    downloadResume: async (id) => bus.setPaused(id, false),
    onDownloadProgress: bus.onProgress,
    onDownloadReport: bus.onReport,
  };
}
