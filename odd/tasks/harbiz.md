# Harbiz platform (local-only)
Objective: download the owner's Harbiz workouts (past and scheduled) into the Lumen library, incrementally.
Branch: `local/harbiz` (never pushed, never merged to main). Delivery: single local branch. Runner: `cargo test -p lumen-harbiz`, `cd ui && pnpm test`.

## Specs
- S1: "quiero que le agregues la funcionalidad a lumen para harbiz con ODD"
- S2: "esta funcionalidad no la quiero en el repo solo en local" (assumption: a local branch that is never pushed; no push, no PR.)
- S3: "tambien quiero tener la opción de descargar los entrenamientos de fechas actualizadas que hayan sido programadas y que no tenga de forma local" (past and scheduled workouts; items already on disk are skipped.)
- S4: "bajar mis cursos desde mayo hasta hoy con los entrenamientos que he realizado" (67 completed = 62 workouts + 5 bodyPhoto check-ins.)
- S5 (assumption): sign in with the email/password from `secure/harbiz` over Meteor DDP (`wss://app.harbiz.io/websocket`); persist only the Meteor resume token, never the password.
- S6 (assumption): layout `Harbiz/Harbiz Training/<YYYY-MM>/<YYYY-MM-DD - title>/` with `workout.json`, `workout.md` and `NN. exercise.<ext>` videos (YouTube via yt-dlp); one library group, reusing the Hotmart scan layout.
- S7 (assumption): body photos are out of scope for the first cut.

- S8: "revisa que se indique bien el numero de clases no pueden haber 222" (a class is a workout: the library must count 66, like the download panel; exercise videos live inside their class.)
- S9: "corrige los errores" (495 errors of the last run: ~80% "This video is not available"/removed, the rest "Sign in to confirm you're not a bot".)

## Tasks
- T1 (S5,S6) parsers: inline, done (RED 8 failed/1 passed -> GREEN 10 passed), commit 8e2c00a
- T2 (S3,S5) DDP client (login, resume, paged past + single upcoming call): inline, done (RED 8+1+3 failed -> GREEN 29 passed; live: login ok, 66 workouts, 5 photo check-ins, 2026-06-01..2026-10-31), commit 954284b
- T3 (S3,S6) planner + downloader with per-file skip: inline, done (RED 10+13 failed -> GREEN 53 passed), commit d1dc945
- T4 (S1) CoursePlatform impl + course listing with local/pending counts: inline, done (RED 7 failed -> GREEN 60 passed), commit 0fb3ade
- T5 (S6) lumen-core LibraryPlatform::Harbiz + scanner + delete target: inline, done (RED compile error + non-exhaustive match -> GREEN core 79, library_delete 14), commit dfbd304
- T6 (S5) Tauri service + commands: inline, done (RED 7 failed -> GREEN lumen 196 passed, clippy clean), commit eca3378
- T7 (S1,S3) UI flow (agenda card shows workouts / new to download / date range; Update re-runs incrementally), i18n es+en, mock, screenshots reviewed at 1280x800 and 760x520: inline, done (RED 8 failed -> GREEN vitest 357 passed, svelte-check 0 errors), commit c0178d8
- T8 (S2) local install from the branch: inline, done (release build from repo root; installed ~/.local/bin/lumen, previous binary kept as lumen.prev; binary contains harbiz commands; real window/download NOT exercised), commit TBD

- T9 (S6,S8) layout: class = workout. `Harbiz Training/<YYYY-MM>/<date - title>/NN. exercise.mp4`, `Description.html`, hidden `.workout.json`; old flat folders are moved into their month: inline, done (RED compile errors -> GREEN harbiz 64, core scanner test proves 3 classes for 5 videos, vitest 362), commit 298b485
- T10 (S9) failures: unavailable videos are warnings (reused lumen_youtube::reason::classify), repeated videos are asked once and reused, run stops after 3 bot checks in a row with one error, 1.5 s pace, full youtu.be link in the report: inline, done (RED compile error -> GREEN harbiz 70, workspace rust 765), commit bd719b9

## Log
- L1 (2026-10-30): "quiero que le agregues la funcionalidad a lumen para harbiz con ODD esta funcionalidad no la quiero en el repo solo en local, tambien quiero tener la opción de descargar los entrenamientos de fechas actualizadas que hayan sido programadas y que no tenga de forma local"
- L2: API map (read-only probe): `getAllCalEventsClient(dir,{$date})` max 50 per call, dir -1 past / 1 next, page back with the oldest date; event = {_id,type:workout|bodyPhoto,title,date,completed,workout{exercises[{name_provisional,url_provisional,sets,target,rest,notes,result}],resultGeneral}}.
- L3: route: all tasks inline (no Writer trigger: units are sequential and depend on each other).
- L4: live findings: the server ignores the cursor going forward (dir=1 always answers with what is next), so only the past is paged and a full upcoming page is an error; rustls needs an explicit aws-lc-rs provider because the crate alone links none (found only by the live run).
- L5: layout decision: module = workout folder `<date> - <title>`, lessons = loose exercise videos `NN. <name>.<ext>` + `NN. <name>.html` note (the scanner already reads loose videos and `<stem>.html` this way, and unnumbered folders sort by name, i.e. by date). Folder names depend only on a workout's own date/title so new scheduled workouts never rename old ones. Metadata (`workout.json`, notes) is rewritten every run so a scheduled workout gains its results later; videos are skipped when present. Kept a private `remove_part_files` copy instead of touching shared `lumen-download` (two copies already exist; keeps the local-only change out of shared crates).
- L6 (2026-10-30): owner: "olvidaste colocar los losgos de harbiz ... agrega la funcionalidad de pausar las descargas en todas las plataformas". Logos: the placeholder H was replaced by the official `harbiz-icon.png` (converted to webp, stage #e35c49), also used as the course-row cover. Pause: built on branch `feat/pause-downloads` (odd/tasks/pause-downloads.md) and merged here; Harbiz joined it with a checkpoint before each workout and each exercise video (RED 1 failed -> GREEN harbiz 61, app 198).
- L7 (2026-10-30): findings: yt-dlp confirms `UNAVAILABLE_ID` is really unavailable, while `WORKING_ID` (reported as bot check) downloads fine from the CLI, so the bot check is a rate effect of ~590 requests in a burst. The library counted exercise videos as lessons (222); the downloader counted workouts (66).
