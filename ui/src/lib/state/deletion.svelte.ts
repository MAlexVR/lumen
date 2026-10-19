// Delete-from-this-computer flow: preview, confirm (trash or permanent),
// result. The library rescans on the library://changed event.
import { deleteTarget, initialDelete, reduceDelete, toDeleteError, type DeleteAction, type DeleteState } from "../logic/deletion";
import { formatBytes } from "../logic/deletion";
import { flattenCourse } from "../logic/progress";
import { afterLessonDelete, lessonDeleteTarget } from "../logic/videos";
import { displayTitle } from "../logic/title";
import type { Route } from "../logic/route";
import type { Course, Group } from "../types";
import { i18n, t } from "../i18n/index.svelte";
import { app } from "./app.svelte";
import { navigate, router } from "./router.svelte";

class DeletionStore {
  state = $state<DeleteState>(initialDelete);
  /** Confirmation shown after a successful delete. */
  notice = $state<string | null>(null);
  /** What to do once the lesson being deleted is gone. */
  #lesson: { group: string; key: string; order: string[]; course: Route } | null = null;
  #noticeTimer: ReturnType<typeof setTimeout> | undefined;

  dispatch(action: DeleteAction): void {
    this.state = reduceDelete(this.state, action);
  }

  async open(group: Group, course?: Course): Promise<void> {
    const api = app.api;
    if (!api) return;
    const target = deleteTarget(group, course);
    const title = displayTitle(group.platform === "skool" && course ? course.title : group.title);
    this.dispatch({ type: "open", target, title });
    try {
      this.dispatch({ type: "previewed", preview: await api.libraryDeletePreview(target) });
    } catch (e) {
      this.dispatch({ type: "failed", error: toDeleteError(e) });
    }
  }

  /** Delete one video, found by group id and lesson key (never a path). */
  async openLesson(groupId: string, key: string): Promise<void> {
    const api = app.api;
    const group = app.group(groupId);
    const course = group?.courses.find((c) => c.sections.some((sec) => sec.lessons.some((l) => l.key === key)));
    const lesson = course?.sections.flatMap((sec) => sec.lessons).find((l) => l.key === key);
    if (!api || !group || !course || !lesson) return;
    const loose = group.collection === "standalone";
    this.#lesson = {
      group: group.id,
      key,
      order: flattenCourse(course)
        .filter((f) => f.lesson.hasVideo || f.lesson.key === key)
        .map((f) => f.lesson.key),
      course: loose ? { name: "library", platform: "youtube" } : { name: "course", group: group.id, course: course.index },
    };
    const target = lessonDeleteTarget(group, key);
    this.dispatch({ type: "open", target, title: displayTitle(lesson.title) });
    try {
      this.dispatch({ type: "previewed", preview: await api.libraryDeletePreview(target) });
    } catch (e) {
      this.dispatch({ type: "failed", error: toDeleteError(e) });
    }
  }

  async confirm(permanent: boolean): Promise<void> {
    const api = app.api;
    const s = this.state;
    if (!api || (s.step !== "confirming" && s.step !== "error")) return;
    this.dispatch({ type: "confirm", permanent });
    if (this.state.step !== "deleting") return;
    const isLesson = !!this.state.target.lesson;
    try {
      const result = await api.libraryDelete(this.state.target, permanent);
      this.dispatch({ type: "deleted", result });
      const size = formatBytes(result.bytesFreed, result.bytesApprox, i18n.locale);
      const lesson = isLesson ? this.#lesson : null;
      this.#notify(t(lesson ? (result.movedToTrash ? "del_done_video_trash" : "del_done_video_permanent") : result.movedToTrash ? "del_done_trash" : "del_done_permanent", { size }));
      if (lesson) {
        const r = router.route;
        const next = afterLessonDelete({
          group: lesson.group,
          order: lesson.order,
          deleted: lesson.key,
          playing: r.name === "watch" && r.group === lesson.group ? r.key : null,
          groupRemoved: result.groupRemoved,
          course: lesson.course,
        });
        if (next) navigate(next);
        this.#lesson = null;
      }
    } catch (e) {
      this.dispatch({ type: "failed", error: toDeleteError(e) });
    }
  }

  close(): void {
    this.dispatch({ type: "close" });
  }

  #notify(text: string): void {
    this.notice = text;
    clearTimeout(this.#noticeTimer);
    this.#noticeTimer = setTimeout(() => (this.notice = null), 6000);
  }
}

export const deletion = new DeletionStore();
