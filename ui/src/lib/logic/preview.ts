// Inicio hero preview: a still frame of the real lesson video, falling back
// to the generated poster until (and unless) a frame is available.
import { mediaUrl } from "../api";
import type { Lesson } from "../types";
import { resumeSeek } from "./time";

/** Share of the duration shown when there is no resume position. */
const EARLY_FRAME = 0.1;

/** Frame to show: the resume position when worth resuming (same rule as the
 * player), otherwise an early representative frame. */
export function previewTime(resumeSeconds: number, duration: number): number {
  if (!Number.isFinite(duration) || duration <= 0) return 0;
  return resumeSeek(resumeSeconds, duration) || duration * EARLY_FRAME;
}

/** Media server URL of the lesson video, or null when there is nothing to load. */
export function previewSrc(mediaBase: string, lesson: Pick<Lesson, "video" | "hasVideo">): string | null {
  return mediaBase && lesson.hasVideo && lesson.video ? mediaUrl(mediaBase, lesson.video) : null;
}

export function previewFace(s: { src: string | null; ready: boolean; failed: boolean }): "video" | "poster" {
  return s.src && s.ready && !s.failed ? "video" : "poster";
}
