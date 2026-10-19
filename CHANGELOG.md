# Changelog

All notable changes to Lumen. Format based on [Keep a Changelog](https://keepachangelog.com/); versions follow semver.
Each release's notes and its git tag message are taken from the matching section (see `scripts/changelog-section.sh`).

## [Unreleased]

### Added
- Delete a single video from the library (any platform), moved to the trash by default.

### Changed
- YouTube videos and playlists are shown as videos, not courses: 16:9 video cards, playlist cards and a flat playlist page.

## [0.3.4] - 2026-10-19

### Added
- In-app updates: *Ajustes → Actualizaciones* downloads the new version, verifies its SHA-256 and its signature, installs it and restarts Lumen, replacing the installed version.
  It detects how Lumen is installed and updates that install in place: rpm (`dnf`) and deb (`apt`) packages through the system administrator prompt, AppImage by replacing the file, and the Windows installer in passive mode.
  Copies installed by hand are not modified: the verified file is downloaded and the screen shows where it is.
- Release installers are signed (a `.sig` is published next to each file) and the app only installs a version whose signature matches the public key built into it.

### Changed
- The updates screen shows how the app is installed and what the update will do before asking for confirmation.

### Note
- The first self-updating version is 0.3.4; install it once by hand (`sudo dnf install ./Lumen-0.3.4-1.x86_64.rpm`). Automatic updates start with the following release.

## [0.3.3] - 2026-10-18

### Changed
- The repository is public; the README no longer requires GitHub sign-in to download.
- Platform logo chips (Biblioteca tabs and group headers) share one size, with an optical scale per platform.
- The README gallery uses only YouTube screenshots with invented data.
- All sample data, test fixtures and mocks use invented courses, creators and lessons; no real third-party course content ships in the repository or in the app.
- The repository was recreated with a fresh history. Earlier pre-releases (0.2.0 to 0.3.2) were withdrawn; their entries below are kept as history.

## [0.3.2] - 2026-10-18

### Added
- Cached video thumbnails (generated once with ffmpeg) on every poster: Inicio shelves and hero, Biblioteca grids and the course header.
  They refresh by themselves when videos are added, changed or deleted.
- Muted 5-second video preview when hovering or focusing a card (one at a time, never on touch screens or with reduced motion).
- Settings: check for updates against the GitHub releases (optional access token while the repository is private).
- `CHANGELOG.md`, and release notes generated from it.

### Changed
- Biblioteca loads much faster: cards no longer mount one video player each.
- CI and release workflows use current GitHub Action versions and pin `ubuntu-24.04`.
- Screenshots are generated locally and no longer committed; a curated gallery lives in `docs/images/`.

### Fixed
- The release workflow failed at the publish step (asset folder layout, then a name clash with the repository's `assets/` folder).

## [0.3.1] - 2026-10-17

### Added
- Poster cards in Inicio and Biblioteca preview the real course videos (one video player per card).
- Official YouTube logo on the picker tile and library chips.

### Known issue
- Biblioteca could be slow with many courses; fixed in 0.3.2.

## [0.3.0] - 2026-10-17

### Added
- Pluralsight downloader (cookie login, course history), YouTube downloader (single videos and playlists, quality presets),
  and a guided Udemy connection check that reports which courses are downloadable (DRM is never bypassed; no Udemy downloads yet).
- Inicio hero shows a real frame of the lesson video.
- First Linux and Windows 11 installers: `.rpm`, `.deb`, `.AppImage` and an NSIS installer that bundles yt-dlp and ffmpeg.

## [0.2.0] - 2026-10-17

### Added
- Rust and Tauri 2 rewrite with a new interface and brand.
- Hotmart and Skool downloaders (Skool native videos included), a download report for every course, and a safer `.complete` rule.
- Multi-platform library, classroom player with per-module progress, library folder picker and safe course deletion (trash by default).
