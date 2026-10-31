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

## Tasks
- T1 (S5,S6) parsers: inline, done (RED 8 failed/1 passed -> GREEN 10 passed), commit 8e2c00a
- T2 (S3,S5) DDP client (login, resume, paged past + single upcoming call): inline, done (RED 8+1+3 failed -> GREEN 29 passed; live: login ok, 66 workouts, 5 photo check-ins, 2026-06-01..2026-10-31), commit 954284b
- T3 (S3,S6) planner + downloader with per-file skip: inline, done (RED 10+13 failed -> GREEN 53 passed), commit d1dc945
- T4 (S1) CoursePlatform impl + course listing with local/pending counts: inline, done (RED 7 failed -> GREEN 60 passed), commit TBD
- T5 (S6) lumen-core LibraryPlatform::Harbiz + scanner: inline, pending
- T6 (S5) Tauri service + commands: inline, pending
- T7 (S1) UI flow, i18n, mock: inline, pending
- T8 (S2) local install from the branch: inline, pending

## Log
- L1 (2026-10-30): "quiero que le agregues la funcionalidad a lumen para harbiz con ODD esta funcionalidad no la quiero en el repo solo en local, tambien quiero tener la opción de descargar los entrenamientos de fechas actualizadas que hayan sido programadas y que no tenga de forma local"
- L2: API map (read-only probe): `getAllCalEventsClient(dir,{$date})` max 50 per call, dir -1 past / 1 next, page back with the oldest date; event = {_id,type:workout|bodyPhoto,title,date,completed,workout{exercises[{name_provisional,url_provisional,sets,target,rest,notes,result}],resultGeneral}}.
- L3: route: all tasks inline (no Writer trigger: units are sequential and depend on each other).
- L4: live findings: the server ignores the cursor going forward (dir=1 always answers with what is next), so only the past is paged and a full upcoming page is an error; rustls needs an explicit aws-lc-rs provider because the crate alone links none (found only by the live run).
- L5: layout decision: module = workout folder `<date> - <title>`, lessons = loose exercise videos `NN. <name>.<ext>` + `NN. <name>.html` note (the scanner already reads loose videos and `<stem>.html` this way, and unnumbered folders sort by name, i.e. by date). Folder names depend only on a workout's own date/title so new scheduled workouts never rename old ones. Metadata (`workout.json`, notes) is rewritten every run so a scheduled workout gains its results later; videos are skipped when present. Kept a private `remove_part_files` copy instead of touching shared `lumen-download` (two copies already exist; keeps the local-only change out of shared crates).
