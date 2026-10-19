// YouTube part of the preview mock. Resolve rules: any non-link -> invalid_url,
// other sites -> unsupported_url, "PRIVADO" in the link -> unavailable,
// `list=` -> a six-item playlist (one private), otherwise a single video.
// Presets `?youtube=noytdlp|downloading|report-warn|nolibrary`.
import type { Api } from "../api";
import { youtubeHint } from "../logic/youtube";
import type { DownloadError, Group, Lesson, PlatformLibrary, YoutubeItem, YoutubePreview } from "../types";
import type { createBus } from "./downloadsMock";

type YoutubeApi = Pick<Api, "youtubeResolve" | "youtubeDownloadStart" | "youtubeDownloadCancel" | "youtubeDownloadsRunning">;

const err = (code: DownloadError["code"], message: string): DownloadError => ({ code, message });

const PLAYLIST_ITEMS: YoutubeItem[] = [
  { id: "a1b2c3d4e5f", title: "Afinación y postura", durationSecs: 612, thumbnailUrl: null, available: true, reason: null },
  { id: "b2c3d4e5f6g", title: "Primeros acordes: Mi menor y Do", durationSecs: 845, thumbnailUrl: null, available: true, reason: null },
  { id: "Xp4Mq9vR2tA", title: "Ritmo básico con púa", durationSecs: null, thumbnailUrl: null, available: false, reason: "private video" },
  { id: "c3d4e5f6g7h", title: "Cambios de acorde sin pausas", durationSecs: 931, thumbnailUrl: null, available: true, reason: null },
  { id: "d4e5f6g7h8i", title: "Arpegios para principiantes", durationSecs: 1204, thumbnailUrl: null, available: true, reason: null },
  { id: "e5f6g7h8i9j", title: "Tu primera canción completa", durationSecs: null, thumbnailUrl: null, available: true, reason: null },
];

function preview(url: string): YoutubePreview {
  const hint = youtubeHint(url);
  if (hint.state === "not_url" || hint.state === "empty") throw err("invalid_url", "not a link");
  if (hint.state !== "valid") throw err("unsupported_url", "not a YouTube video or playlist link");
  if (url.includes("PRIVADO")) throw err("unavailable", "private video");
  if (hint.kind === "playlist") {
    const id = new URL(url.includes("://") ? url : `https://${url}`).searchParams.get("list") ?? "PLmock";
    return {
      kind: "playlist",
      id,
      title: "Curso de guitarra acústica desde cero",
      channel: "Taller de Cuerdas",
      thumbnailUrl: null,
      durationSecs: null,
      folder: "Curso de guitarra acústica desde cero",
      items: structuredClone(PLAYLIST_ITEMS),
    };
  }
  return {
    kind: "video",
    id: "dQw4w9WgXcQ",
    title: "Cómo grabar voz en casa con un solo micrófono",
    channel: "Estudio Abierto",
    thumbnailUrl: null,
    durationSecs: 1386,
    folder: "Videos sueltos",
  };
}

export function createYoutubeMock(preset: string | null, bus: ReturnType<typeof createBus>): YoutubeApi {
  const PLAYLIST_URL = "https://www.youtube.com/playlist?list=PLguitarra01";
  if (preset === "downloading")
    bus.simulate("youtube", PLAYLIST_URL, PLAYLIST_URL, { startAt: 2, total: 5, modules: ["Curso de guitarra acústica desde cero", "Curso de guitarra acústica desde cero"] });
  if (preset === "report-warn") bus.finished("youtube", PLAYLIST_URL, PLAYLIST_URL, "ytwarn", 6);

  return {
    youtubeResolve: async (url) => {
      await new Promise((r) => setTimeout(r, 350));
      if (preset === "noytdlp") throw err("ytdlp_missing", "yt-dlp was not found");
      return preview(url);
    },
    youtubeDownloadStart: async (request) => {
      if (preset === "noytdlp") throw err("ytdlp_missing", "yt-dlp was not found");
      if (preset === "nolibrary") throw err("no_library", "choose a library folder first");
      const p = preview(request.url);
      const key = p.kind === "video" ? `https://www.youtube.com/watch?v=${p.id}` : `https://www.youtube.com/playlist?list=${p.id}`;
      if (bus.isRunning("youtube", key)) throw err("already_running", "already running");
      const total = p.kind === "video" ? 1 : (request.items?.length ?? PLAYLIST_ITEMS.length);
      return bus.simulate("youtube", key, key, { total, outcome: p.kind === "video" ? "ok" : "ytwarn", modules: [p.folder, p.folder] });
    },
    youtubeDownloadCancel: async (id) => bus.cancel(id, "youtube"),
    youtubeDownloadsRunning: async () => bus.runningOf("youtube"),
  };
}

/** Downloaded YouTube groups for the mock library (folders match the layout). */
export function youtubeLibrary(): PlatformLibrary {
  const lesson = (folder: string, n: number, title: string, audio = false): Lesson => {
    const dir = `YouTube/${folder}/${String(n).padStart(2, "0")} - ${title}`;
    return {
      key: `youtube:1-${folder}/1-${folder}/${String(n).padStart(2, "0")}-${title}`,
      title,
      index: n,
      video: `${dir}/${title}.${audio ? "m4a" : "mp4"}`,
      description: `${dir}/description.txt`,
      hasVideo: true,
      resources: [],
      thumbnail: null,
    };
  };
  const group = (folder: string, lessons: Lesson[], collection: "playlist" | "standalone" = "playlist"): Group => ({
    id: `youtube/${folder}`,
    platform: "youtube",
    name: folder,
    title: folder,
    seller: null,
    status: "complete",
    lessonCount: lessons.length,
    collection,
    courses: [{ title: folder, index: 1, sections: [{ title: folder, index: 1, lessons }] }],
  });
  const playlist = "Curso de guitarra acústica desde cero";
  return {
    id: "youtube",
    groups: [
      group(playlist, [
        lesson(playlist, 1, "Afinación y postura"),
        lesson(playlist, 2, "Primeros acordes: Mi menor y Do"),
        lesson(playlist, 4, "Cambios de acorde sin pausas"),
        lesson(playlist, 5, "Arpegios para principiantes"),
        lesson(playlist, 6, "Tu primera canción completa"),
      ]),
      group(
        "Videos sueltos",
        [
          lesson("Videos sueltos", 1, "Cómo grabar voz en casa con un solo micrófono"),
          lesson("Videos sueltos", 2, "Entrevista: el oficio del luthier", true),
          lesson("Videos sueltos", 3, "Paseo sonoro por el mercado de la ciudad"),
        ],
        "standalone",
      ),
    ],
  };
}
