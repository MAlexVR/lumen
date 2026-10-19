# AGENTS.md — how to work on Lumen

Read this first. It applies to any coding agent (Codex, Claude Code, others). The owner speaks Spanish; keep chat answers short and in Spanish. Everything written into the repository (code, comments, docs, commit messages, UI keys) is **English**, except user-facing UI strings which go through i18n (es first, en second).

Project goal, architecture and stages: `docs/PROPOSAL.md`. Current state and open work: `docs/HANDOFF.md`. Task log with evidence and the command/event contracts: `odd/tasks/lumen.md` (append to it, do not rewrite history).

## Non-negotiables

1. **Strict TDD.** Write the failing test first, watch it fail (RED), implement, watch it pass (GREEN), then refactor. Report the RED and GREEN counts. No network in tests: use fixtures, fakes, and local mock servers (`wiremock` or a tiny test server). Tests that need the owner's real library are `#[ignore]`d and read it only.
2. **Never fail silently.** Every skipped or failed item becomes an issue in the download report (`lesson`, `kind`, `severity`, `host`, redacted `url`, `message`). `.complete` is written only when there is no error-severity issue. Silent skipping is what made the previous app produce empty folders.
3. **Secrets.** Cookies, tokens and sessions are never logged, never put in URLs of reports (strip query strings), never committed, never in test fixtures. Do not read `.env` or cookie files. Session files live in the app data dir with restricted permissions.
4. **No DRM circumvention.** Do not build anything that bypasses Widevine/PlayReady or other technical protection. If a course is DRM-protected, report it as `video_not_resolved` with a clear reason.
5. **Destructive actions are recoverable.** Deleting library content goes to the OS trash by default; permanent deletion needs a separate confirmation. Paths are rebuilt from validated components and confined to the library root. Never accept raw paths from the UI.
6. **Do not touch the owner's library** (`~/Vídeos/Lumen`) from tests or scripts except read-only `#[ignore]` tests. Use temp dirs.
7. **Git.** One work-unit commit per coherent step, Conventional Commits (`feat(scope):`, `fix(scope):`, `refactor:`, `test:`, `docs:`, `chore:`). **No `Co-Authored-By` and no AI attribution lines.** `git add` explicit paths (never `git add -A`). Do not push unless the owner asks. Tests and docs ship in the same commit as the behaviour.
8. **Be honest in reports.** Say what was verified (with the command and result) and what was not (live services, Windows, real window). Never claim a check you did not run.

## Architecture (hexagonal)

```
crates/lumen-core      domain + ports, no Tauri/reqwest: types, scanner, progress store (SQLite), CoursePlatform trait, report/progress types
crates/lumen-download  platform-agnostic: yt-dlp runner, file streaming (.part -> rename), retry/backoff, report writer, folder layout helpers, Windows-safe sanitizer
crates/lumen-skool     Skool adapter (parsers, client, planner, downloader)
crates/lumen-hotmart   Hotmart adapter (same shape)
crates/lumen-media     token-protected loopback media server (Range, Host check, confinement)
src-tauri/             thin Tauri glue: commands delegate to testable AppCore-style structs; events
ui/                    Svelte 5 (runes) + TypeScript strict + Vite
```

- A platform = a crate implementing `CoursePlatform` with: pure parsers over JSON/HTML (fixture-tested), a client (retry/backoff, cookie/token scoping, redacted `Debug`), a planner (what to download and where), a downloader built on `lumen-download`, and thin `<platform>_*` commands + shared events in `src-tauri`.
- Keep the same shape as `lumen-skool` and `lumen-hotmart` (copy their module layout and naming). Do not duplicate what `lumen-download` already provides; extend it instead.
- Command contract (match existing ones): `<platform>_login_*`, `<platform>_session_status`, `<platform>_logout`, `<platform>_courses|groups`, `<platform>_download_start`, `<platform>_download_cancel`, `<platform>_downloads_running`. Errors are `{code, message}`. Events: `download://progress` and `download://report` (additive fields only, always carry `platform` and `id`).
- JSON is camelCase and additive; never break existing UI consumers without migrating them in the same change.
- On disk: `<library>/<Platform>/<course or collection>/<NN - module>/<NN - lesson>/…` (see `crates/lumen-download/src/layout.rs`). Folder names go through the shared Windows-safe sanitizer (reserved names, trailing dots/spaces, `MAX_PATH` truncation).

## Rust style

- `clippy -D warnings` and `cargo fmt --check` clean. No `unwrap()`/`expect()` outside tests. Errors with `thiserror`, typed and mapped to stable codes at the Tauri boundary.
- Small focused modules and functions; names say what, comments say why (sparse). No dead code, no speculative abstractions, no commented-out code.
- Pure functions for parsing/planning so they are trivially testable; I/O at the edges.
- Async with `tokio`; honour cancellation (`CancelFlag`); never block the async runtime with blocking I/O.
- Match surrounding code before inventing patterns.

## UI style (Svelte 5 + TS)

- Runes only (`$state`, `$derived`, `$effect`, `$props`). TypeScript strict. Pure logic lives in plain `.ts` modules with `vitest` tests (RED→GREEN); components stay thin.
- **Design system is normative** (`docs/PROPOSAL.md`, `ui/src/lib/design/`): true black stage, warm ivory text, ONE champagne-gold accent (`--lumen`), moonstone as quiet counterpoint, titles in Sora, UI text in Geist (bundled locally, no CDN), hairlines instead of heavy boxes, light as the motif (soft glow only on the active/continue element). Use tokens only: **no hardcoded colours**, no emoji as icons (use the inline SVG icon set), no purple/blue gradients, no generic card grids with drop shadows.
- Accessibility: WCAG 2.2 AA, semantic landmarks, `focus-visible`, `aria-*` for progress/sliders/disclosures, never colour alone (pair with text/icon), `prefers-reduced-motion`. Dialogs are native `<dialog>` with focus on the safe action.
- i18n: all visible strings externalised (es default, en). Map every backend error code to a message that says what happened and what to do next.
- `ui/src/lib/api.ts` is the only place that calls Tauri; it has a mock implementation for non-Tauri previews (`ui/src/lib/mock/`), so the UI can be screenshotted headlessly.
- **Look at your work.** Regenerate screenshots with `scripts/ui-shots.sh`, open every PNG, iterate at least twice, `docs/screenshots/` is generated and gitignored; never commit frames of the owner's real course videos. The curated, mock-only gallery used by the README is `docs/images/`. Check 1280x800, 1024x640 and 760x520.

## Commands

```bash
cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check
cd ui && pnpm install && pnpm test && pnpm check && pnpm build
cd ui && pnpm tauri dev                          # live development
pnpm tauri build --no-bundle                     # release binary with the UI embedded (target/release/lumen)
pnpm tauri build --debug --no-bundle             # debug binary with the UI embedded
cargo test -p lumen-core --test real_library -- --ignored   # reads ~/Vídeos/Lumen, read-only; set LUMEN_SAMPLE_GROUP
```

Gotchas: a plain `cargo build` produces a binary that loads the dev server (`localhost:1420`) and shows a blank page — always use `pnpm tauri build`. `generate_context!` embeds `ui/dist`, so build the UI before `cargo check` in CI. Do not run the app binary just to test flags: it opens a window.

Local install (owner's machine, user-level, no sudo): binary at `~/.local/bin/lumen`, `~/.local/share/applications/lumen.desktop` (`StartupWMClass=lumen`), icons in `~/.local/share/icons/hicolor/<size>/apps/lumen.png`. Replace the binary atomically (`install` to `lumen.new`, then `mv`).

## Releases

Versioning is semver in `Cargo.toml` (workspace), `src-tauri/tauri.conf.json`, `package.json` and `ui/package.json`. Every release needs a `CHANGELOG.md` section: it becomes the release notes and the tag message (`git tag -a vX.Y.Z --cleanup=verbatim -F <(scripts/changelog-section.sh X.Y.Z) && git push origin vX.Y.Z (`--cleanup=verbatim` keeps the `###` headings, which git would otherwise strip as comments)`). Installers are built only for tagged versions, after tests pass, and are published as release assets — never committed. Windows 11 is the primary target but is not testable on the Linux dev machine: Windows CI is a manual job and the owner tests real Windows builds.
