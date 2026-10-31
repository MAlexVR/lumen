// Turns a burst of "please run this" requests into few runs: never two at once,
// at most one every `minIntervalMs`, and one trailing run so the last request
// is never lost. Used to rescan the library while downloads keep finishing.

export interface Coalescer {
  request(): void;
}

export function createCoalescer(run: () => Promise<void>, minIntervalMs: number): Coalescer {
  let running = false;
  let pending = false;
  let lastStart = Number.NEGATIVE_INFINITY;
  let timer: ReturnType<typeof setTimeout> | null = null;

  const start = () => {
    timer = null;
    running = true;
    pending = false;
    lastStart = Date.now();
    let outcome: Promise<void>;
    try {
      outcome = run();
    } catch {
      outcome = Promise.resolve();
    }
    void outcome
      .catch(() => {})
      .finally(() => {
        running = false;
        if (pending) schedule();
      });
  };

  const schedule = () => {
    if (running || timer !== null) return;
    const wait = Math.max(0, lastStart + minIntervalMs - Date.now());
    if (wait === 0) start();
    else timer = setTimeout(start, wait);
  };

  return {
    request() {
      pending = true;
      schedule();
    },
  };
}
