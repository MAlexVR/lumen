# Lumen — proposal

## Goals
1. Rewrite the core in **Rust** for speed and a single code base.
2. Ship for **Windows 11** (primary) and Linux (dev/test) from one project.
3. A distinctive, cinematic UI/UX instead of a generic dashboard.
4. One installer that already bundles `yt-dlp` and `ffmpeg` (Tauri sidecars).

## Architecture (hexagonal)
```
lumen/
  crates/lumen-core    domain + ports: Course/Module/Lesson, CoursePlatform trait,
                       LibraryScanner, ProgressStore, DownloadEngine (traits, no I/O)
  crates/lumen-skool   Skool adapter: HTML __NEXT_DATA__ parsing, Mux native videos
  crates/lumen-media   loopback media server (axum, Range, per-run token), thumbnails
  src-tauri/           app shell: Tauri commands, events, sidecars, window effects
  ui/                  Svelte 5 + Vite + TypeScript, local fonts, no CDN
```
- Platforms implement one trait; the UI never knows platform details.
- Library index + progress in SQLite (FTS5 search over titles/descriptions) replaces localStorage.
- Existing on-disk layout is kept, so current downloads keep working.

## Speed
- tokio with bounded concurrency (default 3 lessons in parallel), metadata prefetch while downloading,
  resumable downloads (`.part` + HTTP Range), HTTP/2 via reqwest.
- `yt-dlp` stays the engine for external hosts. Native HLS (Skool/Mux) in Rust is a later phase, only if measurements justify it.

## Windows 11 specifics
- Installer: NSIS per-user (no admin). WebView2 ships with Windows 11; installer bootstraps it on older Windows 10.
- Look and feel: Mica window effect + custom title bar (snap layouts kept), follows system theme/accent contrast settings.
- Paths: default library in the user's Videos folder; handle MAX_PATH (260) with long-path prefix and
  truncation of very long course/lesson names; reject reserved names (CON, NUL...) and trailing dots/spaces.
- Build: GitHub Actions `windows-latest` (cannot be validated from the Linux dev machine; a real Windows 11 test is required
  before each release). Unsigned builds trigger SmartScreen until a code-signing certificate is added.

## Design direction ("black and light")
Lumen means light: a true-black stage where the content glows. Elegant and professional, no neon, no purple/blue gradients.

Palette (contrast ratios computed against WCAG 2.2; AA needs 4.5:1 for text):

| Token | Hex | Use | on #000 |
|---|---|---|---|
| `--bg` | `#000000` | app background (true black) | |
| `--surface-1` | `#0B0B0D` | sidebar, panels | |
| `--surface-2` | `#141417` | cards | |
| `--surface-3` | `#1D1D21` | hover, inputs | |
| `--line` | `#26262B` | hairline borders (decorative) | 1.4 |
| `--text` | `#F3EFE6` | primary text (warm ivory) | 18.3 |
| `--text-muted` | `#A39E92` | secondary text | 7.9 |
| `--text-faint` | `#6E6A61` | decorative/disabled ONLY | 3.9 (not for text) |
| `--lumen` | `#D9B76A` | the accent: champagne gold (primary action, progress, focus) | 10.9 |
| `--lumen-hover` | `#EBCB85` | accent hover | 13.4 |
| `--lumen-glow` | `rgba(217,183,106,.18)` | soft radiance behind key elements | |
| `--moon` | `#93A8C6` | secondary accent: moonstone blue-gray (links, info, in-progress) | 8.7 |
| `--ok` | `#82BC8F` | completed | 9.5 |
| `--danger` | `#E5766E` | errors | 7.1 |

Rules: ONE warm accent (`--lumen`) per view; `--moon` is the quiet counterpoint. Text on a `--lumen` button is `#000000` (10.9:1).
Light is the motif: a subtle radial glow behind the "continue" hero and the active lesson, never full-surface gradients.
Never rely on colour alone (pair with icon/text).

Other direction:
- Type: a serif display face for titles + a neutral grotesk for UI, bundled locally.
- Course cards as posters: large type, cover or generated still frame (ffmpeg), thin gold progress line.
- Home: hero "continue where you left off" with a real video still and resume time, shelves per course.
- Player: custom controls, theatre mode, speed, keyboard shortcuts, chapter list, remembers position.
- Motion: purposeful, short, `prefers-reduced-motion` respected.

## Stages
- S0 Repo + foundation (this commit).
- S1 Library, progress (SQLite), media server, design system, Home/Library/Player reading the existing library.
- S2 Skool downloader in Rust; login through an in-app webview (real browser engine, passes the WAF).
- S3 Parallel engine, resume, thumbnails.
- S4 Windows 11 CI build + installer; manual test on a real Windows 11 machine.
- S5 Port the remaining platforms by priority (Hotmart first).

## Risks
- No Windows machine here: Windows behaviour is unverified until tested by the owner.
- Tauri's cookie access for in-app login must be confirmed on both WebView2 and WebKitGTK (spike in S2).
