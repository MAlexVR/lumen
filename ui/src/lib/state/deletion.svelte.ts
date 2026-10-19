// Delete-from-this-computer flow: preview, confirm (trash or permanent),
// result. The library rescans on the library://changed event.
import { deleteTarget, initialDelete, reduceDelete, toDeleteError, type DeleteAction, type DeleteState } from "../logic/deletion";
import { formatBytes } from "../logic/deletion";
import { displayTitle } from "../logic/title";
import type { Course, Group } from "../types";
import { i18n, t } from "../i18n/index.svelte";
import { app } from "./app.svelte";

class DeletionStore {
  state = $state<DeleteState>(initialDelete);
  /** Confirmation shown after a successful delete. */
  notice = $state<string | null>(null);
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

  async confirm(permanent: boolean): Promise<void> {
    const api = app.api;
    const s = this.state;
    if (!api || (s.step !== "confirming" && s.step !== "error")) return;
    this.dispatch({ type: "confirm", permanent });
    if (this.state.step !== "deleting") return;
    try {
      const result = await api.libraryDelete(this.state.target, permanent);
      this.dispatch({ type: "deleted", result });
      const size = formatBytes(result.bytesFreed, result.bytesApprox, i18n.locale);
      this.#notify(t(result.movedToTrash ? "del_done_trash" : "del_done_permanent", { size }));
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
