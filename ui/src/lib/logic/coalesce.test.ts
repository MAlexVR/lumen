import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createCoalescer } from "./coalesce";

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

const deferred = () => {
  let resolve!: () => void;
  const promise = new Promise<void>((r) => (resolve = r));
  return { promise, resolve };
};

describe("createCoalescer", () => {
  it("runs the first request at once", async () => {
    const run = vi.fn(async () => {});
    createCoalescer(run, 8000).request();
    expect(run).toHaveBeenCalledTimes(1);
  });

  it("turns a burst of requests into one trailing run after the interval", async () => {
    const run = vi.fn(async () => {});
    const c = createCoalescer(run, 8000);
    c.request();
    await vi.advanceTimersByTimeAsync(1000);
    c.request();
    c.request();
    c.request();
    expect(run).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(6999);
    expect(run).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(run).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(20000);
    expect(run).toHaveBeenCalledTimes(2);
  });

  it("never runs two at once: a request during a run reruns after it, not before", async () => {
    const first = deferred();
    const run = vi.fn().mockReturnValueOnce(first.promise).mockResolvedValue(undefined);
    const c = createCoalescer(run, 1000);
    c.request();
    await vi.advanceTimersByTimeAsync(5000);
    c.request();
    await vi.advanceTimersByTimeAsync(5000);
    expect(run).toHaveBeenCalledTimes(1);
    first.resolve();
    await vi.advanceTimersByTimeAsync(0);
    expect(run).toHaveBeenCalledTimes(2);
  });

  it("keeps working after a run fails", async () => {
    const run = vi.fn().mockRejectedValueOnce(new Error("disk busy")).mockResolvedValue(undefined);
    const c = createCoalescer(run, 1000);
    c.request();
    await vi.advanceTimersByTimeAsync(0);
    await vi.advanceTimersByTimeAsync(1000);
    c.request();
    await vi.advanceTimersByTimeAsync(0);
    expect(run).toHaveBeenCalledTimes(2);
  });

  it("runs again at once when the last run was long ago", async () => {
    const run = vi.fn(async () => {});
    const c = createCoalescer(run, 1000);
    c.request();
    await vi.advanceTimersByTimeAsync(60000);
    c.request();
    expect(run).toHaveBeenCalledTimes(2);
  });
});
