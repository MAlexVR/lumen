// One session (timer and sets done) per workout, kept while the app is open so
// moving between exercises, or leaving and coming back, does not lose it.
import { newSession, type WorkoutSession } from "../logic/workoutSession";

class WorkoutSessions {
  #all = $state<Record<string, WorkoutSession>>({});

  get(lessonKey: string): WorkoutSession {
    return this.#all[lessonKey] ?? newSession();
  }

  update(lessonKey: string, change: (session: WorkoutSession) => WorkoutSession): void {
    this.#all[lessonKey] = change(this.get(lessonKey));
  }

  reset(lessonKey: string): void {
    delete this.#all[lessonKey];
  }
}

export const sessions = new WorkoutSessions();
