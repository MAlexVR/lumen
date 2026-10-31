# Library refreshes while a download queue is running
Objective: a class that finished downloading shows up in the library while the rest of the queue is still downloading, without blocking or crashing the app.
Branch: `feat/live-library-refresh` (from `feat/pause-downloads`, not pushed). Runner: `cd ui && pnpm test && pnpm check`.

## Specs
- S1: "me gustaría que cuando una clase o entrenamiento se descargue se pueda ver en la biblioteca mientras la cola se termina sin que la app se bloquee o crashee"
- S2 (assumption): a lesson counts as finished when a progress event reports more completed lessons than the UI had. Today the library is rescanned only on the final report (`ui/src/lib/state/downloads.svelte.ts`).
- S3 (assumption): rescans are coalesced: never two at once, at most one every 8 s, and one trailing rescan so the last lesson is not missed. The scan already runs off the UI thread (`spawn_blocking`) and ignores `.part` files.

## Tasks
- T1 (S3) coalescing runner (pure, fake timers): inline, done (RED module missing -> GREEN 5 tests)
- T2 (S2) lesson-finished detection (`finishedALesson`) + wiring in `initDownloads`/`app.rescanSoon`: inline, done (RED 5 failed -> GREEN vitest 368, svelte-check 0 errors); wiring covered by the app-level rescanSoon test, not by a real download

## Log
- L1 (2026-10-30): "me gustaría que cuando una clase o entrenamiento se descargue se pueda ver en la biblioteca mientras la cola se termina sin que la app se bloquee o crashee"
- L2: route: inline (two small dependent units).
