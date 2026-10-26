# Handoff — state and next work

Read `AGENTS.md` first (rules and style are normative). Contracts and evidence per task: `odd/tasks/lumen.md`.

## State (Lumen 0.3.0)

Built and verified on Linux (Fedora, WebKitGTK) with fixtures and fakes; **not verified**: live Skool/Hotmart/Pluralsight/YouTube/Udemy (Golden Life was checked live, see below), the native folder dialog, the real trash UI, Windows 11.

- Platforms in code: Hotmart, Skool, Pluralsight, Golden Life and YouTube (videos and playlists) work end to end against fakes. Golden Life (`crates/lumen-goldenlife`, Cademí LMS, email + password form login, Panda Video HLS and YouTube embeds) was also exercised against the live academy with `cargo run -p lumen-goldenlife --example live -- <credentials-file> survey|download` (spec and evidence: `odd/tasks/goldenlife.md`). Udemy has a connection wizard and a diagnostic only (no downloader yet).
- UI: Inicio (continue hero with a real video frame + shelves), Biblioteca (platform tabs, posters, delete), Curso (per-module progress), Reproductor, Descargas (platform picker + per-platform flows), Ajustes (library folder).
- Tests at the last integration: `cargo test --workspace` 521 passed, UI `pnpm test` 285 passed.

### Disk layout (verified against the real library)

`<library>/<Platform>/<course or collection>/…`, library default `~/Vídeos/Lumen`:

```
Skool/<group>/<NN - course>/<NN - section>/<NN - lesson>/{<title>.mp4, description.html, files, Attachments/}
Hotmart/<course> - <seller>/<N. module>/<N. lesson>/{N. Lesson.mp4, Description.html, Materials/…}
```

Rule for every new platform: **one folder per platform, one folder per course/collection inside it**. The scanner (`crates/lumen-core`) must understand each layout and return the uniform tree `Group > Course > Section > Lesson`; lesson `key`s and group ids are platform-qualified so progress never collides (see how Hotmart did it).

## Open work

### 1. Udemy phase 2: downloader (blocked on real data)

Phase 1 is done: `crates/lumen-udemy` (tolerant client, parsers on **assumed** fixtures, DRM classifier, sanitizer) plus the connection/diagnostic wizard. Every endpoint and field is a hypothesis until the owner runs the wizard with a real PERSONAL account and shares `udemy-diagnostico.json` (tokens, email, names and titles are redacted). Then:

- Replace the assumed fixtures with sanitized real shapes, fix parsers/client, and build the downloader on `lumen-download` for **non-DRM** lectures only (video, captions, resources, articles). DRM lectures stay reported as `drm_protected`; never bypass.
- Layout: `Udemy/<course>/<NN - section>/<NN - lecture>/…`; extend the scanner and the Library platform list (platform-qualified ids/keys), delete/open-folder support, then enable Descargar in the wizard result.
- Contract and unverified list: `odd/tasks/lumen.md` (S9.udemy-1, S9.udemy-ui).

### 2. Small gaps

- No backend command opens the app data folder, so the Udemy wizard offers "Copiar ruta" instead of "Abrir carpeta".
- Real thumbnails (ffmpeg, cached) for posters; parallel downloads and resume (S3).
- Windows 11: manual CI job, then a real-machine test before any release.

## Verification checklist for the owner after each step

1. `cargo test --workspace`, clippy, fmt, `pnpm test/check/build` green.
2. `pnpm tauri build --no-bundle`, install to `~/.local/bin/lumen` (see `AGENTS.md`), open from the launcher.
3. Try the real flow (login → list → download one item), then check `download-report.json` in the course folder for anything unexpected and send it back.
4. Windows 11: run the manual `windows` job in CI and test the installer on a real machine before any release.

## Open questions for the owner

- YouTube defaults: max quality, subtitles (download `.vtt`/embed).
- Pluralsight: a current subscription is needed to verify against the live service.
