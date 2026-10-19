import { describe, expect, it } from "vitest";
import type { Group, GroupProgress, Lesson } from "../types";
import { afterLessonDelete, lessonDeleteTarget, videoPreviewFor, videoState, wording, youtubeSections } from "./videos";

const lesson = (key: string, title = key, video = true, thumbnail: string | null = null): Lesson => ({
  key,
  title,
  index: 1,
  video: video ? `YouTube/G/${key}/v.mp4` : null,
  description: null,
  hasVideo: video,
  resources: [],
  thumbnail,
});
const group = (id: string, collection: "playlist" | "standalone", lessons: Lesson[], platform: Group["platform"] = "youtube"): Group => ({
  id,
  platform,
  name: id,
  title: id,
  seller: null,
  status: "complete",
  lessonCount: lessons.length,
  collection,
  courses: [{ title: id, index: 1, sections: [{ title: id, index: 1, lessons }] }],
});
const loose = group("youtube/Videos sueltos", "standalone", [lesson("a", "Grabar voz"), lesson("b", "Luthier")]);
const list = group("youtube/Guitarra", "playlist", [lesson("g1", "Afinación"), lesson("g2", "Acordes")]);
const skool = group("Skool G", "playlist", [lesson("s1")], "skool");

describe("youtubeSections", () => {
  it("makes each standalone lesson its own video and keeps playlists apart", () => {
    const s = youtubeSections([loose, list, skool], "");
    expect(s.videos.map((v) => v.lesson.key)).toEqual(["a", "b"]);
    expect(s.videos[0].group.id).toBe("youtube/Videos sueltos");
    expect(s.playlists.map((g) => g.id)).toEqual(["youtube/Guitarra"]);
  });

  it("filters videos by title and playlists by title or video title", () => {
    expect(youtubeSections([loose, list], "luthier").videos.map((v) => v.lesson.key)).toEqual(["b"]);
    expect(youtubeSections([loose, list], "acordes").playlists).toHaveLength(1);
    expect(youtubeSections([loose, list], "zzz")).toEqual({ videos: [], playlists: [] });
  });
});

describe("wording", () => {
  it("says video for YouTube and clase for everything else", () => {
    expect(wording("youtube")).toMatchObject({ count: "videos", of: "video_of", countOf: "videos_count" });
    expect(wording("skool")).toMatchObject({ count: "lessons", of: "lesson_of", countOf: "lessons_count" });
    expect(wording("hotmart").count).toBe("lessons");
  });
});

describe("videoPreviewFor", () => {
  const progress: GroupProgress = { watched: [], lastLesson: "a", positions: [{ lessonKey: "a", seconds: 42 }] };
  it("builds the hover preview with the resume position, or null without a video or server", () => {
    expect(videoPreviewFor(lesson("a"), progress, "http://127.0.0.1:9/m/t")).toEqual({
      src: "http://127.0.0.1:9/m/t/YouTube/G/a/v.mp4",
      path: "YouTube/G/a/v.mp4",
      resumeAt: 42,
    });
    expect(videoPreviewFor(lesson("a", "a", false), progress, "http://127.0.0.1:9/m/t")).toBeNull();
    expect(videoPreviewFor(lesson("a"), progress, "")).toBeNull();
  });
});

describe("lessonDeleteTarget", () => {
  it("uses the group id and the lesson key, never a path", () => {
    expect(lessonDeleteTarget(loose, "a")).toEqual({ platform: "youtube", group: "youtube/Videos sueltos", lesson: "a" });
  });
});

describe("afterLessonDelete", () => {
  const order = ["a", "b", "c"];
  const course = { name: "course", group: "g", course: 1 } as const;
  it("goes to the library when the whole group went away", () => {
    expect(afterLessonDelete({ group: "g", order, deleted: "b", playing: "b", groupRemoved: true, course })).toEqual({ name: "library", platform: null });
  });
  it("stays put when the deleted video was not the open one", () => {
    expect(afterLessonDelete({ group: "g", order, deleted: "c", playing: "a", groupRemoved: false, course })).toBeNull();
    expect(afterLessonDelete({ group: "g", order, deleted: "c", playing: null, groupRemoved: false, course })).toBeNull();
  });
  it("moves a playing video to the next one, else the previous, else the course page", () => {
    expect(afterLessonDelete({ group: "g", order, deleted: "b", playing: "b", groupRemoved: false, course })).toEqual({ name: "watch", group: "g", key: "c" });
    expect(afterLessonDelete({ group: "g", order, deleted: "c", playing: "c", groupRemoved: false, course })).toEqual({ name: "watch", group: "g", key: "b" });
    expect(afterLessonDelete({ group: "g", order: ["a"], deleted: "a", playing: "a", groupRemoved: false, course })).toEqual(course);
  });
});

describe("videoState", () => {
  it("is done when watched, in progress past 5 s, else new", () => {
    expect(videoState(true, 0)).toBe("done");
    expect(videoState(false, 42)).toBe("progress");
    expect(videoState(false, 3)).toBe("new");
  });
});
