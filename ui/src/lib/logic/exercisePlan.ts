import type { ExercisePlan } from "../types";

/** The `NN` of a workout video's file name (`03. Walking lunge.mp4`), if it has one. */
export function exerciseNumber(videoPath: string): number | null {
  const name = videoPath.split("/").pop() ?? videoPath;
  const match = /^(\d+)\.\s/.exec(name);
  return match ? Number(match[1]) : null;
}

/** The plan of the exercise a video belongs to, if the workout has one for it. */
export function planFor(plans: readonly ExercisePlan[], videoPath: string): ExercisePlan | null {
  const number = exerciseNumber(videoPath);
  return number === null ? null : (plans.find((p) => p.number === number) ?? null);
}

/** `10` -> `10 reps`, `8-12` -> `8-12 reps`; anything with a unit (`45 s`) stays as written. */
export function targetLabel(target: string, repsWord: string): string {
  const text = target.trim();
  return /^[\d-]+$/.test(text) ? `${text} ${repsWord}` : text;
}

/** The name to show for an exercise: the coach's, or the title taken from the video file. */
export function exerciseDisplayName(plan: ExercisePlan | null, videoTitle: string): string {
  const named = plan?.name.replace(/\s{2,}/g, " ").trim();
  return named || videoTitle.trim();
}
