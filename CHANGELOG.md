# Changelog

All notable changes to Lumen. Format based on [Keep a Changelog](https://keepachangelog.com/); versions follow semver.
Each release's notes and its git tag message are taken from the matching section (see `scripts/changelog-section.sh`).

## [0.6.4] - 2026-10-31

This is the first release since 0.4.1: it ships everything listed under 0.5.0 to 0.6.3 below. The highlights:

- **Harbiz**, a new download platform for your coaching workouts: past and scheduled, one class per workout grouped by month, each exercise a video with its own instructions, incremental updates, and honest reports (a video that no longer exists is a warning, and repeated YouTube bot checks stop the run with one clear message).
- **Pause and resume** any running download on every platform, and finished classes show up in the library while the rest of a queue is still downloading.
- **Player**: *play from the start* and *repeat* buttons, a lessons panel that lists the videos of the playing lesson (foldable), next/previous that walk through a lesson's videos, and full screen that survives moving to the next video.
- **Workout mode** (Harbiz): over the video, the exercise name and instructions, a timer that counts only while the video plays, and set-by-set progress with its repetitions.

### Added
- The instructions of the exercise on screen now appear under its name, in the top-left corner of the video, in the same quiet translucent style (Harbiz). A small button next to the name hides or shows them, and the choice is remembered. They come from the coach's notes already saved with each class, so nothing needs to be downloaded again.

## [0.6.3] - 2026-10-31

### Fixed
- Moving to the next exercise (or the next lesson) while in full screen no longer leaves full screen. The player used to be rebuilt for every video, and the browser exits full screen when its element goes away; now the same player switches video in place. Volume is kept between videos too.
- Because the player is reused, every position saved or "watched" mark is filed under the video that was really playing, not the one the screen has moved to.

## [0.6.2] - 2026-10-31

### Changed
- The workout timer now follows the video: it counts only while a video plays, stops when you pause it (or leave the player) and resumes when you press play, so rests with the video paused are no longer counted. Clicking the timer still pauses or resumes it by hand.

## [0.6.1] - 2026-10-31

### Added
- The name of the exercise on screen now appears in the top-left corner of the video (Harbiz), in the same quiet style as the workout timer and the set readout. It uses the name the coach gave the exercise, or the title from the video file when there is none, and fades back with the rest of the readout when you stop moving the mouse.

## [0.6.0] - 2026-10-31

### Added
- **Workout readout over the video** (Harbiz): a small, quiet panel in the top-right corner of the player shows a timer for the whole workout and the set you are on with its repetitions (`Serie 2/3 · 10 reps`). The check button (or the `S` key) marks the set done and moves to the next one; after the last set it shows *Completado* with a *Siguiente ejercicio* button. A small undo button takes back a set.
- The workout timer starts the first time a video of the workout plays and keeps running through the rests. Click it to pause or resume; while paused a button restarts the workout (timer and sets). Sets and time are kept while the app is open, so moving between exercises does not lose them.
- While an exercise still has sets left, its video plays again when it ends instead of moving on, so automatic *Next* cannot skip an exercise you have not finished. Once its sets are done it advances as usual.
- Sets, repetitions and rest per exercise are read from the metadata Lumen already keeps in each Harbiz class folder (`.workout.json`); no new download is needed.

## [0.5.4] - 2026-10-31

### Added
- The playing lesson in the player's lessons panel can be folded: a chevron next to its menu hides or shows the list of its videos (the exercises of a workout). While folded it shows which video is playing (`3/7`). The choice is remembered.

## [0.5.3] - 2026-10-31

### Fixed
- *Repeat* in the player now lets the video play to its very end before starting over. It used the browser's own loop, which restarted early and showed a clipped video; Lumen now waits for the real end and replays it itself.

## [0.5.2] - 2026-10-30

### Added
- In the player, the lesson being played now lists its videos (the exercises of a Harbiz workout) under its name in the lessons panel; click one to play it.
- When Lumen starts, the notes of Harbiz workouts already on disk are rewritten from their saved metadata (no network, no update needed): each exercise video gets its own note and the class page no longer lists every exercise.

### Changed
- Each Harbiz video is tied to its exercise by the full name, not just by the number. If the coach renames or reorders an exercise, or gives it another video, the old file is moved to a hidden `.replaced/` folder inside the class (nothing is deleted) and the right video is fetched. A name cut short because the library lives on a deeper path still matches its exercise. Exercises that share a video download it once and each keeps its own file and note.

### Fixed
- The context menu of a lesson in the player's list no longer shows underneath the rows that follow it.

## [0.5.1] - 2026-10-30

### Added
- The player has a *repeat* button (key `R`) that keeps the current video looping until you move on; the choice is remembered.
- A lesson with several videos (a Harbiz workout: one per exercise) now steps through them: *Next* and *Previous* (and autoplay) go to the next exercise first and to the next lesson after the last one. The line under the title says which video it is (`Video 2 de 5 · Exercise name`).

### Changed
- While a video plays, only its own note is shown: for a Harbiz exercise, that exercise's instructions, sets and results (`NN. name.html` next to the video). The class page (`Description.html`) now holds the workout summary and the exercises that have no video, instead of listing every exercise.
- The "Video adicional" list under the player is gone; extra videos are reached with *Next*/*Previous*. A lesson now marks itself watched when its last video is watched.

## [0.5.0] - 2026-10-30

### Added
- **Harbiz** (the coaching app) is a new download platform. Sign in with your Harbiz email and password; Lumen keeps only the session token. Your agenda, past and scheduled workouts, is saved as `Harbiz/Harbiz Training/<YYYY-MM>/<date - title>/`: one class per workout, with each exercise video (`NN. exercise.mp4`), a `Description.html` with how the workout went and every exercise's sets, notes and results. *Update* only downloads what is not on disk yet, so workouts scheduled later are picked up on the next run. A video that no longer exists on YouTube is reported as a warning (with the reason and the full link) instead of failing the course, a video shared by several workouts is fetched once, and the run stops after three YouTube bot checks in a row instead of listing hundreds of errors.
- **Pause and resume** any running download, on every platform. The item in flight finishes and the queue waits at the next one; cancelling still works while paused, and the paused state survives reloading the app.
- Finished classes now appear in the library while the rest of a download queue is still running. The library is rescanned at most once every few seconds, so a long queue does not slow the app down; Harbiz publishes a class only once it has finished downloading.
- The player has a *play from the start* button (keys `0` or `Home`), and its controls are grouped: play/pause, start, ±10 s, then volume and time.

### Fixed
- A course whose modules are unnumbered folders (such as Harbiz's month folders) now opens from the library instead of staying on the same screen, and its modules are numbered by position.

## [0.4.1] - 2026-10-25

### Fixed
- Pressing Download on a Golden Life course no longer freezes the app ("Lumen is not responding").
- The Golden Life logo now shows in the platform picker on Linux (WebKit did not draw its transparent image).
- The platform connect screens no longer show the Lumen logo above "Connect your account".

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
