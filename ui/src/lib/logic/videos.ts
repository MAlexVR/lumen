// YouTube is not a course platform: standalone videos and playlists are shown
// as videos. Wording by platform, the video list of the "Videos sueltos" group,
// lesson delete targets and where to go after deleting one. Pure.
import type { Course, DeleteTarget, Group, GroupProgress, Lesson, PlatformId } from "../types";
import { previewSrc } from "./preview";
import { positionOf } from "./progress";
import type { Route } from "./route";
import { normalize } from "./search";
import { stripEmoji } from "./title";

export const isYoutube = (platform: PlatformId | string): boolean => platform === "youtube";

/** Message keys for counting things: videos on YouTube, lessons elsewhere. */
export function wording(platform: PlatformId | string) {
  return isYoutube(platform)
    ? ({ count: "videos", of: "video_of", countOf: "videos_count", title: "videos_title", ofTotal: "of_videos" } as const)
    : ({ count: "lessons", of: "lesson_of", countOf: "lessons_count", title: "lessons", ofTotal: "of_lessons" } as const);
}

export interface VideoItem {
  group: Group;
  course: Course;
  lesson: Lesson;
}

const matches = (text: string, words: string[]) => {
  const hay = normalize(`${text} ${stripEmoji(text)}`);
  return words.every((w) => hay.includes(w));
};

/** YouTube groups as the library shows them: every lesson of the standalone
 * group is its own video; every other group is a playlist. */
export function youtubeSections(groups: readonly Group[], query: string): { videos: VideoItem[]; playlists: Group[] } {
  const words = normalize(query).split(" ").filter(Boolean);
  const mine = groups.filter((g) => isYoutube(g.platform));
  const videos = mine
    .filter((g) => g.collection === "standalone")
    .flatMap((group) =>
      group.courses.flatMap((course) =>
        course.sections.flatMap((s) => s.lessons.map((lesson) => ({ group, course, lesson }))),
      ),
    )
    .filter((v) => matches(v.lesson.title, words));
  const playlists = mine
    .filter((g) => g.collection !== "standalone")
    .filter((g) => matches([g.title, ...g.courses.flatMap((c) => c.sections.flatMap((s) => s.lessons.map((l) => l.title)))].join(" "), words));
  return { videos, playlists };
}

/** Hover preview of one video (resume position included), or null. */
export function videoPreviewFor(
  lesson: Lesson,
  progress: GroupProgress,
  mediaBase: string,
): { src: string; path: string; resumeAt: number } | null {
  const src = previewSrc(mediaBase, lesson);
  return src && lesson.video ? { src, path: lesson.video, resumeAt: positionOf(progress, lesson.key) } : null;
}

export const lessonDeleteTarget = (group: Group, key: string): DeleteTarget => ({
  platform: group.platform,
  group: group.id,
  lesson: key,
});

/** Where to go once a lesson is gone: nowhere unless it was the open one;
 * then the next video, else the previous one, else the course page; the
 * library when the whole group disappeared. */
export function afterLessonDelete(i: {
  group: string;
  order: readonly string[];
  deleted: string;
  playing: string | null;
  groupRemoved: boolean;
  course: Route;
}): Route | null {
  if (i.groupRemoved) return { name: "library", platform: null };
  if (i.playing !== i.deleted) return null;
  const at = i.order.indexOf(i.deleted);
  const rest = i.order.filter((k) => k !== i.deleted);
  const key = rest[at] ?? rest[at - 1];
  return key ? { name: "watch", group: i.group, key } : i.course;
}

/** Watched state of one video; a saved position under 5 s does not count as started. */
export function videoState(watched: boolean, seconds: number): "done" | "progress" | "new" {
  if (watched) return "done";
  return seconds >= 5 ? "progress" : "new";
}
