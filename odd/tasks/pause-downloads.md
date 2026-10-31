# Pause and resume downloads (all platforms)
Objective: pause a running download and resume it later, on every platform that downloads (Skool, Hotmart, Pluralsight, Golden Life, YouTube; Harbiz joins when this merges into `local/harbiz`).
Branch: `feat/pause-downloads` (from main, not pushed). Runner: `cargo test --workspace`, `cd ui && pnpm test && pnpm check`.

## Specs
- S1: "agrega la funcionalidad de pausar las descargas en todas las plataformas"
- S2 (assumption): cooperative pause. The flag is checked before each item (lesson/video/file); the item in flight finishes, then the run holds until resumed or cancelled. Reason: workspace forbids `unsafe`, Windows has no safe process suspend, and cancelling yt-dlp deletes its `.part` files, so stop-and-restart would lose progress.
- S3 (assumption): Cancel keeps working while paused. A paused run still counts as running (no second download of the same group).
- S4 (assumption): one generic `download_pause` / `download_resume` command pair (by download id) instead of one pair per platform.
- S5 (assumption): `RunningDownload` gains an additive `paused` field so a reloaded UI restores the state.

## Tasks
- T1 (S2,S3) core: pause flag on CancelFlag + `wait_if_paused` in lumen-download: inline, done (core RED compile error -> GREEN 80; wait_if_paused 3 tests written with the implementation, no separate RED), commit 443b2e9
- T2 (S1,S2) per-item checkpoints in skool/hotmart/pluralsight/goldenlife/youtube downloaders: inline, done (RED 5 failed -> GREEN hotmart 45, goldenlife 59, skool 75, pluralsight 46, youtube 35), commit 99ad4cf
- T3 (S3,S4,S5) DownloadManager pause/resume + running().paused + generic commands: inline, pending
- T4 (S1) UI: pause/resume buttons, state, i18n es+en, mock: inline, pending

## Log
- L1 (2026-10-30): "agrega la funcionalidad de pausar las descargas en todas las plataformas"
- L2: route: all tasks inline (sequential, each depends on the previous; no Writer trigger).
- L3: explorer map: CancelFlag is `Arc<AtomicBool>` created per download in `DownloadManager::start_task` (`src-tauri/src/downloads.rs`) and passed to every `download_group`; yt-dlp cancel kills the child and cleans `.part` (`crates/lumen-download/src/ytdlp.rs`); Hotmart/GoldenLife delete `*.part` at run start.
