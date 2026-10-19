// App-wide thumbnail broker: cards say when they are on screen, requests go
// out in batches (50 ms debounce, 24 per call), one listener resolves
// thumbnails://ready, and a version counter makes lookups reactive.
import { ThumbQueue, type ThumbItem, type ThumbLookup } from "../logic/thumbs";
import { app } from "./app.svelte";

const BATCH = 24;
const DEBOUNCE_MS = 50;
const WIDTH = 480;
const DEFAULT_AT = 8;

/** Moment of the thumbnail: the resume position when meaningful, else 8 s. */
export const thumbAt = (resumeAt: number): number => (resumeAt >= 5 ? Math.floor(resumeAt) : DEFAULT_AT);

class ThumbBroker {
  version = $state(0);
  /** Bumped when thumbnails must be re-requested (library changed). */
  generation = $state(0);
  ffmpegMissing = $state(false);
  #queue = new ThumbQueue();
  #timer: ReturnType<typeof setTimeout> | undefined;
  #started = false;

  async init(): Promise<void> {
    const api = app.api;
    if (!api || this.#started) return;
    this.#started = true;
    // Downloads, deletions and folder changes all end in a rescan.
    app.afterRescan(() => this.invalidate());
    await api.onThumbnailReady((e) => {
      this.#queue.applyReady(e);
      this.version += 1;
    });
    const status = await api.thumbnailsStatus().catch(() => null);
    this.ffmpegMissing = status?.ffmpegAvailable === false;
  }

  want(item: ThumbItem): void {
    this.#queue.want(item);
    this.#schedule();
  }

  leave(item: ThumbItem): void {
    this.#queue.leave(item);
    this.#schedule();
  }

  /** Library content changed: revalidate what visible cards show. */
  invalidate(paths?: readonly string[]): void {
    this.#queue.invalidate(paths);
    this.generation += 1;
  }

  /** An <img> failed to load: poster for good, and forget the entry. */
  fail(item: ThumbItem): void {
    this.#queue.fail(item);
    this.version += 1;
  }

  lookup(item: ThumbItem): ThumbLookup {
    void this.version;
    return this.#queue.lookup(item);
  }

  #schedule(): void {
    clearTimeout(this.#timer);
    this.#timer = setTimeout(() => void this.#flush(), DEBOUNCE_MS);
  }

  async #flush(): Promise<void> {
    const api = app.api;
    if (!api) return;
    const cancels = this.#queue.takeCancels();
    if (cancels.length) void api.thumbnailsCancel(cancels).catch(() => {});
    for (let batch = this.#queue.takeBatch(BATCH); batch.length; batch = this.#queue.takeBatch(BATCH)) {
      try {
        const items = batch.map(({ path, atSecs }) => ({ path, atSecs }));
        this.#queue.applyResults(await api.thumbnailsRequest(items, batch[0].width ?? WIDTH));
      } catch {
        this.#queue.applyResults(batch.map((i) => ({ ...i, state: "unavailable" as const })));
      }
      this.version += 1;
    }
  }
}

export const thumbs = new ThumbBroker();
