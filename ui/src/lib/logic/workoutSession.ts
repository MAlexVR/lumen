// A workout in progress: one timer for the whole workout and, per exercise, how
// many sets are done. Pure functions over plain data (timestamps are passed in)
// so they are easy to test; the player keeps one session per workout.

export interface WorkoutSession {
  /** Milliseconds counted before the timer last (re)started. */
  accumulated: number;
  /** When the timer last started running; null while paused or before the first video. */
  runningSince: number | null;
  /** The first video played, so the timer has begun at least once. */
  started: boolean;
  /** Sets done, by exercise number. */
  sets: Readonly<Record<number, number>>;
}

export const newSession = (): WorkoutSession => ({ accumulated: 0, runningSince: null, started: false, sets: {} });

/** Starts the timer the first time a video of the workout plays; later calls change nothing. */
export function startTimer(s: WorkoutSession, now: number): WorkoutSession {
  return s.started ? s : { ...s, started: true, runningSince: now };
}

/** Pauses a running timer or resumes a paused one. */
export function toggleTimer(s: WorkoutSession, now: number): WorkoutSession {
  if (!s.started) return s;
  return s.runningSince === null
    ? { ...s, runningSince: now }
    : { ...s, accumulated: s.accumulated + (now - s.runningSince), runningSince: null };
}

export const elapsedMs = (s: WorkoutSession, now: number): number =>
  s.accumulated + (s.runningSince === null ? 0 : Math.max(0, now - s.runningSince));

export function formatElapsed(ms: number): string {
  const total = Math.floor(Math.max(0, ms) / 1000);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const sec = String(total % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${sec}` : `${m}:${sec}`;
}

export interface SetProgress {
  done: number;
  /** The set being worked on (the last one once finished). */
  current: number;
  total: number | null;
  finished: boolean;
}

export function setProgress(s: WorkoutSession, exercise: number, total: number | null): SetProgress {
  const done = s.sets[exercise] ?? 0;
  const finished = total !== null && done >= total;
  return { done, current: finished ? total : done + 1, total, finished };
}

/** Marks the current set done, moving to the next; never past the last set. */
export function completeSet(s: WorkoutSession, exercise: number, total: number | null): WorkoutSession {
  const done = s.sets[exercise] ?? 0;
  if (total !== null && done >= total) return s;
  return { ...s, sets: { ...s.sets, [exercise]: done + 1 } };
}

export function undoSet(s: WorkoutSession, exercise: number): WorkoutSession {
  const done = s.sets[exercise] ?? 0;
  return done === 0 ? s : { ...s, sets: { ...s.sets, [exercise]: done - 1 } };
}

/** Stay on the exercise (replay its video) while it has sets left, so autoplay cannot skip it. */
export const shouldHold = (progress: SetProgress | null): boolean =>
  progress !== null && progress.total !== null && !progress.finished;

export const resetSession = (): WorkoutSession => newSession();
