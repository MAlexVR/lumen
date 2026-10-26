# Changelog

All notable changes to Lumen. Format based on [Keep a Changelog](https://keepachangelog.com/); versions follow semver.
Each release's notes and its git tag message are taken from the matching section (see `scripts/changelog-section.sh`).

## [0.4.0] - 2026-10-25

### Added
- **Golden Life** (Golden Life Academy, built on the Cademí platform) is a new download platform. Sign in with the academy email and password; Lumen keeps only the session (the password is never stored). Every course is listed by academy section with its cover, and each lesson is saved as `GoldenLife/<course>/<N. section>/<N. lesson>/` with its video (Panda Video, plus embedded YouTube videos), `Description.html` and `Materials/` attachments. A lesson the platform itself shows without a video is reported as a warning instead of failing the course. The Golden Life logo appears in the platform picker and the library.

### Fixed
- On Windows a video no longer fails with "[WinError 32] … 'Lesson.temp.mp4' -> 'Lesson.mp4'" when antivirus or the search indexer holds the file at the moment yt-dlp renames it. The download is already complete, so Lumen now finishes the rename itself, retrying for up to 30 seconds, instead of marking the lesson as failed.

## [0.3.11] - 2026-10-24

### Changed
- A course that comes back without modules (Hotmart, Pluralsight) now shows a clear message in the download result, with what to do next (reconnect the session and retry), instead of "unexpected response: course has no modules".

## [0.3.10] - 2026-10-24

### Fixed
- A video download that fails after the stream was downloaded (for example ffmpeg blocked by Windows "Controlled folder access") no longer leaves the raw file under its final name. Before, the next run treated it as already downloaded.
- Windows no longer flashes a console window each time a thumbnail is generated on the Home screen.

## [0.3.9] - 2026-10-24

### Fixed
- Downloads on every platform now check that the library folder can be created and written before starting. When it cannot (for example Windows "Controlled folder access" blocking `Videos`), the error names the folder and asks to choose another in Settings, instead of "0 of 0 lessons" and a bare "file not found".

## [0.3.8] - 2026-10-24

### Fixed
- Release build: the thumbnail tests no longer trip clippy on Linux. 0.3.6 and 0.3.7 were tagged but never published because of this; 0.3.8 is the first release with the Skool and Hotmart fixes listed below.

## [0.3.7] - 2026-10-24

### Fixed
- Release build: the thumbnail tests stay recognised as test code so clippy passes on Linux. 0.3.6 was tagged but never published because of this; 0.3.7 is the first release with the Skool and Hotmart fixes listed below.

## [0.3.6] - 2026-10-24

### Fixed
- Skool: courses were all skipped as "no access" because Skool now sends `hasAccess` as `1` instead of `true`, which left downloads at "0 of 0 lessons".
- Skool: a failure creating the group folder now names the folder instead of only "file not found".
- Hotmart on Windows: lessons failed with `WinError 32` while renaming the `.part` file. yt-dlp now downloads 4 fragments at a time instead of 64.
- Windows checkouts no longer corrupt the signed update test fixture (`.gitattributes`).

### Changed
- The Windows test suite compiles and passes clippy; Unix-only test helpers are gated.
- Manual live-check examples for Skool, Hotmart and the media server (`cargo run -p ... --example ...`) read credentials only from environment variables.

## [0.3.5] - 2026-10-19

### Added
- Delete a single video from the library (any platform), moved to the trash by default. A playlist left without videos is removed too, so no empty card remains.
- YouTube videos show their own thumbnail (the one downloaded with the video), without needing ffmpeg.

### Changed
- YouTube videos and playlists are shown as videos, not courses: each individual video is its own 16:9 card, playlists have their own cards and a flat numbered page, and the wording says "video" instead of "class", "course" or "module".
- The search box on the YouTube tab says it searches videos and playlists.

### Note
- Update from 0.3.4 from *Ajustes → Actualizaciones* (*Instalar y reiniciar*): this is the first release that 0.3.4 can install by itself.

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
