// Udemy connection wizard state: session, options, live diagnostic progress
// and the last report. The report path is only known after a run in this
// session (udemy_diagnostic_last returns the JSON, not its path).
import { toDownloadError } from "../logic/downloadErrors";
import { initialProgress, initialWizard, reduceProgress, reduceWizard, type ProgressState, type WizardAction, type WizardState } from "../logic/udemy";
import type { DownloadError, UdemyDiagnoseOptions, UdemyReport } from "../types";
import { app } from "./app.svelte";

class UdemyStore {
  wizard = $state<WizardState>(initialWizard);
  ready = $state(false);
  loggingIn = $state<"webview" | "cookies" | null>(null);
  loginStartedAt = $state<number | null>(null);
  error = $state<DownloadError | null>(null);
  options = $state<Required<Omit<UdemyDiagnoseOptions, "maxCourses">> & { maxCourses: number | null }>({
    sampleLecturesPerCourse: 5,
    includeTitles: false,
    maxCourses: null,
  });
  progress = $state<ProgressState>(initialProgress);
  report = $state<UdemyReport | null>(null);
  reportPath = $state<string | null>(null);
  cancelling = $state(false);
  #listening = false;

  dispatch(a: WizardAction): void {
    this.wizard = reduceWizard(this.wizard, a);
  }

  async init(): Promise<void> {
    const api = app.api;
    if (!api || this.ready) return;
    if (!this.#listening) {
      this.#listening = true;
      await api.onUdemyProgress((e) => (this.progress = reduceProgress(this.progress, e)));
    }
    try {
      const [{ loggedIn }, report] = await Promise.all([api.udemySessionStatus(), api.udemyDiagnosticLast()]);
      this.report = report;
      this.dispatch({ type: "resume", loggedIn, hasReport: !!report });
    } catch (e) {
      this.error = toDownloadError(e);
    }
    this.ready = true;
  }

  async login(kind: "webview" | "cookies", text = ""): Promise<boolean> {
    const api = app.api;
    if (!api) return false;
    this.loggingIn = kind;
    this.error = null;
    this.loginStartedAt = kind === "webview" ? Date.now() : null;
    try {
      if (kind === "webview") await api.udemyLoginWebview();
      else await api.udemyLoginCookies(text);
      this.dispatch({ type: "loggedIn" });
      return true;
    } catch (e) {
      this.error = toDownloadError(e);
      return false;
    } finally {
      this.loggingIn = null;
      this.loginStartedAt = null;
    }
  }

  async diagnose(): Promise<void> {
    const api = app.api;
    if (!api) return;
    this.error = null;
    this.progress = initialProgress;
    this.dispatch({ type: "start" });
    const o = this.options;
    try {
      const out = await api.udemyDiagnose({
        sampleLecturesPerCourse: o.sampleLecturesPerCourse,
        includeTitles: o.includeTitles,
        ...(o.maxCourses ? { maxCourses: o.maxCourses } : {}),
      });
      this.reportPath = out.path;
      this.report = await api.udemyDiagnosticLast();
      this.dispatch({ type: "finished" });
    } catch (e) {
      const error = toDownloadError(e);
      this.error = error;
      if (error.code === "session_expired" || error.code === "not_logged_in" || error.code === "bot_challenge") {
        this.dispatch({ type: "sessionLost" });
      } else {
        this.report = await api.udemyDiagnosticLast().catch(() => null);
        this.dispatch({ type: this.report ? "finished" : "back" });
      }
    } finally {
      this.cancelling = false;
    }
  }

  async cancel(): Promise<void> {
    this.cancelling = true;
    const ok = await app.api?.udemyDiagnoseCancel().catch(() => false);
    if (!ok) this.cancelling = false;
  }

  async logout(): Promise<void> {
    await app.api?.udemyLogout().catch(() => {});
    this.dispatch({ type: "sessionLost" });
  }
}

export const udemy = new UdemyStore();
