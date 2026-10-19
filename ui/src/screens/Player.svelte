<script lang="ts">
  import { displayTitle } from "../lib/logic/title";
  import { untrack } from "svelte";
  import Icon from "../lib/design/Icon.svelte";
  import { t } from "../lib/i18n/index.svelte";
  import { flattenCourse, positionOf } from "../lib/logic/progress";
  import { wording } from "../lib/logic/videos";
  import { app } from "../lib/state/app.svelte";
  import { href, navigate } from "../lib/state/router.svelte";
  import Description from "./player/Description.svelte";
  import DeleteDialog from "./library/DeleteDialog.svelte";
  import LessonList from "./player/LessonList.svelte";
  import VideoStage from "./player/VideoStage.svelte";
  import type { Resource } from "../lib/types";

  let { group, lessonKey }: { group: string; lessonKey: string } = $props();

  const flat = $derived(app.flat(group));
  const index = $derived(flat.findIndex((f) => f.lesson.key === lessonKey));
  const item = $derived(index >= 0 ? flat[index] : undefined);
  const prev = $derived(index > 0 ? flat[index - 1] : undefined);
  const next = $derived(index >= 0 && index < flat.length - 1 ? flat[index + 1] : undefined);
  const watched = $derived(app.watchedOf(group));
  const isWatched = $derived(watched.has(lessonKey));
  const courseFlat = $derived(item ? flattenCourse(item.course) : []);
  const position = $derived(courseFlat.findIndex((f) => f.lesson.key === lessonKey) + 1);

  const pref = (key: string, fallback: boolean): boolean => {
    try {
      const v = localStorage.getItem(key);
      return v === null ? fallback : v === "1";
    } catch {
      return fallback;
    }
  };
  const store = (key: string, on: boolean) => {
    try {
      localStorage.setItem(key, on ? "1" : "0");
    } catch {
      /* not persisted */
    }
  };

  let theatre = $state(pref("lumen.theatre", false));
  let autoplayNext = $state(pref("lumen.autoplayNext", true));
  let drawer = $state(false);
  let videoError = $state<number | null>(null);
  /** Lesson reached through autoplay; only that one starts playing by itself. */
  let autoKey = $state<string | null>(null);

  // Saved position captured once per lesson so our own saves never re-seek.
  const resumeAt = $derived.by(() => {
    const key = lessonKey;
    return untrack(() => positionOf(app.progressOf(group), key));
  });

  $effect(() => {
    const key = lessonKey;
    if (!item) return;
    untrack(() => {
      videoError = null;
      altVideo = null;
      reading = null;
      void app.setLast(group, key);
    });
  });

  function go(target: typeof next, auto = false) {
    if (!target) return;
    autoKey = auto ? target.lesson.key : null;
    navigate({ name: "watch", group, key: target.lesson.key });
  }

  function toggleTheatre() {
    theatre = !theatre;
    store("lumen.theatre", theatre);
  }

  function toggleAutoplay() {
    autoplayNext = !autoplayNext;
    store("lumen.autoplayNext", autoplayNext);
  }

  function onEnded() {
    if (!autoplayNext) return;
    const upcoming = flat.slice(index + 1).find((f) => f.lesson.hasVideo);
    go(upcoming, true);
  }

  /** Additional video of this lesson playing instead of the main one. */
  let altVideo = $state<string | null>(null);
  /** Supplementary reading shown inline. */
  let reading = $state<string | null>(null);
  const extraVideos = $derived(item ? item.lesson.resources.filter((r) => r.kind === "video") : []);
  const files = $derived(item ? item.lesson.resources.filter((r) => r.kind !== "video") : []);

  function openResource(r: Resource) {
    if (r.kind === "reading") {
      reading = reading === r.path ? null : r.path;
      return;
    }
    if (r.url) void app.api?.openUrl(r.url);
    else void app.api?.openPath(r.path);
  }
</script>

{#if item}
  {@const lesson = item.lesson}
  <div class="player" class:theatre>
    <div class="main">
      <div class="topbar">
        <a class="back" href={href({ name: "course", group, course: item.course.index })}>
          <Icon name="arrowLeft" size={16} />
          <span>{displayTitle(item.course.title)}</span>
        </a>
        <span class="crumb">{displayTitle(item.section.title)}</span>
        <button
          type="button"
          class="btn btn-quiet drawer-btn"
          aria-expanded={drawer}
          aria-controls="player-lessons"
          onclick={() => (drawer = !drawer)}
        >
          <Icon name="list" size={18} />{t(wording(app.group(group)?.platform ?? "").title)}
        </button>
      </div>

      <div class="screen">
        {#if lesson.hasVideo && lesson.video}
          {#key `${lessonKey}|${altVideo ?? ""}`}
            <VideoStage
              src={app.mediaUrl(altVideo ?? lesson.video)}
              resumeAt={altVideo ? 0 : resumeAt}
              autoplay={autoKey === lessonKey || altVideo !== null}
              {theatre}
              onsave={(s) => {
                if (!altVideo) void app.savePosition(group, lessonKey, s);
              }}
              onwatched={() => {
                if (!altVideo) void app.setWatched(group, lessonKey, true);
              }}
              onended={onEnded}
              onerror={(code) => (videoError = code)}
              onnext={() => go(next)}
              onprev={() => go(prev)}
              ontheatre={toggleTheatre}
            />
          {/key}
          {#if videoError !== null}
            <div class="notice error" role="alert">
              <Icon name="alert" size={22} />
              <div>
                <p class="strong">{t("video_error")}</p>
                <p class="muted num">{t("error_code", { code: videoError })}</p>
              </div>
              <button type="button" class="btn btn-quiet" onclick={() => lesson.video && app.api?.openPath(lesson.video)}>
                <Icon name="external" size={16} />{t("open_system_player")}
              </button>
            </div>
          {/if}
        {:else}
          <div class="novideo">
            <Icon name="noVideo" size={28} />
            <p class="strong">{t("lesson_no_video")}</p>
            <p class="muted">{t("lesson_no_video_hint")}</p>
          </div>
        {/if}
      </div>

      <article class="info">
        <div class="headrow">
          <header>
            <span class="eyebrow num">{t(wording(app.group(group)?.platform ?? "").of, { n: position, total: courseFlat.length })}</span>
            <h1 tabindex="-1" title={displayTitle(lesson.title)}>{displayTitle(lesson.title)}</h1>
          </header>

          <div class="actions">
            <button
              type="button"
              class="btn btn-quiet"
              disabled={!prev}
              onclick={() => go(prev)}
              title="{t('prev')} (P)"
              aria-label={t("prev")}
            >
              <Icon name="prev" size={16} /><span class="lbl">{t("prev")}</span>
            </button>
            <button
              type="button"
              class="btn btn-quiet"
              aria-pressed={isWatched}
              aria-label={t("mark_watched")}
              title={t(isWatched ? "marked_watched" : "mark_watched")}
              onclick={() => void app.setWatched(group, lessonKey, !isWatched)}
            >
              <Icon name="check" size={16} /><span class="lbl">{t(isWatched ? "marked_watched" : "mark_watched")}</span>
            </button>
            <button
              type="button"
              class="btn btn-quiet"
              disabled={!next}
              onclick={() => go(next)}
              title="{t('next')} (N)"
              aria-label={t("next")}
            >
              <span class="lbl">{t("next")}</span><Icon name="next" size={16} />
            </button>
            <span class="spacer"></span>
            <button
              type="button"
              class="switch"
              role="switch"
              aria-checked={autoplayNext}
              aria-label={t("autoplay")}
              title={t("autoplay")}
              onclick={toggleAutoplay}
            >
              <span class="track" aria-hidden="true"><span class="knob"></span></span>
              <span class="lbl">{t("autoplay")}</span>
            </button>
          </div>
        </div>

        {#if extraVideos.length && lesson.video}
          <section class="resources" aria-labelledby="vid-h">
            <h2 id="vid-h" class="eyebrow">{t("res_video")}</h2>
            <ul>
              {#each [{ path: lesson.video, name: t("res_main_video"), main: true }, ...extraVideos.map((v, i) => ({ path: v.path, name: `${t("res_video")} ${i + 1}`, main: false }))] as v (v.path)}
                {@const playing = (altVideo ?? lesson.video) === v.path}
                <li>
                  <button
                    type="button"
                    aria-pressed={playing}
                    onclick={() => (altVideo = v.main ? null : v.path)}
                  >
                    <Icon name="play" size={16} />
                    <span class="rname">{v.name}</span>
                    <span class="rkind">{playing ? t("res_playing") : ""}</span>
                  </button>
                </li>
              {/each}
            </ul>
          </section>
        {/if}

        {#if lesson.description}
          <Description path={lesson.description} />
        {/if}

        {#if reading}
          <Description path={reading} heading={t("res_reading")} />
        {/if}

        {#if files.length}
          <section class="resources" aria-labelledby="res-h">
            <h2 id="res-h" class="eyebrow">{t("resources")}</h2>
            <ul>
              {#each files as r (r.path)}
                <li>
                  <button
                    type="button"
                    onclick={() => openResource(r)}
                    title={r.kind === "drm" ? t("res_drm_hint") : (r.url ?? r.name)}
                    aria-pressed={r.kind === "reading" ? reading === r.path : undefined}
                  >
                    <Icon name={r.url ? "link" : "file"} size={18} />
                    <span class="rname">
                      {r.kind === "reading" ? t("res_reading") : r.name}
                      {#if r.kind === "drm"}<span class="badge">{t("res_drm")}</span>{/if}
                    </span>
                    <span class="rkind">
                      {r.kind === "reading" ? t("description") : t(r.url ? "open_link" : "open_file")}
                    </span>
                  </button>
                </li>
              {/each}
            </ul>
          </section>
        {/if}

        <p class="shortcuts"><span class="eyebrow">{t("shortcuts")}</span> {t("shortcuts_hint")}</p>
      </article>
    </div>

    <aside id="player-lessons" class="side" class:open={drawer} aria-label={t("lessons_toggle")}>
      <button type="button" class="icon-btn close" onclick={() => (drawer = false)} aria-label={t("close")}>
        <Icon name="close" size={18} />
      </button>
      <LessonList {group} course={item.course} currentKey={lessonKey} {watched} onpick={() => (drawer = false)} />
    </aside>
  </div>
{:else}
  <div class="gone" role="status">
    <p>{t("gone_lesson")}</p>
    <a class="btn btn-quiet" href={href({ name: "library", platform: null })}>{t("gone_back")}</a>
  </div>
{/if}

<DeleteDialog onclosed={() => undefined} />

<style>
  .gone {
    display: grid;
    gap: var(--s-4);
    justify-items: start;
    padding: var(--s-16) var(--gutter);
    color: var(--text-muted);
  }
  .gone a {
    text-decoration: none;
  }

  .player {
    display: grid;
    grid-template-columns: minmax(0, 1fr) clamp(280px, 24vw, 360px);
    min-height: 100%;
  }
  .main {
    min-width: 0;
  }
  .topbar {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    height: 56px;
    padding: 0 var(--s-6);
    min-width: 0;
  }
  .back {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    color: var(--text);
    text-decoration: none;
    font-size: var(--fs-sm);
    min-width: 0;
  }
  .back span,
  .crumb {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .back:hover span {
    text-decoration: underline;
  }
  .crumb {
    color: var(--text-muted);
    font-size: var(--fs-sm);
    min-width: 0;
  }
  .crumb::before {
    content: "/";
    color: var(--text-faint);
    margin-right: var(--s-3);
  }
  .drawer-btn {
    margin-left: auto;
    display: none;
    height: 34px;
  }
  .screen {
    position: relative;
    background: var(--bg);
  }
  .notice {
    position: absolute;
    left: 50%;
    top: 50%;
    transform: translate(-50%, -50%);
    display: flex;
    align-items: center;
    gap: var(--s-4);
    padding: var(--s-5) var(--s-6);
    background: var(--surface-1);
    border: 1px solid var(--line);
    border-radius: var(--r-3);
    max-width: calc(100% - var(--s-8));
  }
  .notice > div {
    flex: 1;
    min-width: 200px;
  }
  .notice.error :global(svg) {
    color: var(--danger);
  }
  .notice.error .btn :global(svg) {
    color: inherit;
  }
  .novideo {
    aspect-ratio: 16 / 9;
    width: min(100%, calc((100vh - 168px) * 16 / 9));
    margin-inline: auto;
    display: grid;
    place-content: center;
    justify-items: center;
    gap: var(--s-2);
    color: var(--text-muted);
    background: radial-gradient(50% 60% at 50% 50%, var(--surface-1), var(--bg));
  }
  .strong {
    color: var(--text);
  }
  .muted {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .info {
    display: grid;
    gap: var(--s-8);
    max-width: 920px;
    padding: var(--s-8) var(--s-8) var(--s-20);
  }
  header {
    display: grid;
    gap: var(--s-3);
  }
  h1 {
    font-size: var(--fs-xl);
    line-height: 1.05;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2);
    padding-bottom: var(--s-6);
    border-bottom: 1px solid var(--line);
  }
  .spacer {
    flex: 1;
  }
  .switch {
    display: inline-flex;
    align-items: center;
    gap: var(--s-3);
    font-size: var(--fs-sm);
    color: var(--text-muted);
    height: 40px;
  }
  .track {
    position: relative;
    width: 30px;
    height: 16px;
    border-radius: var(--r-pill);
    border: 1px solid var(--line);
    background: var(--surface-2);
    transition: background-color var(--dur-2) var(--ease-out);
  }
  .knob {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--text-muted);
    transition:
      transform var(--dur-2) var(--ease-out),
      background-color var(--dur-2) var(--ease-out);
  }
  .switch[aria-checked="true"] {
    color: var(--text);
  }
  .switch[aria-checked="true"] .track {
    border-color: var(--lumen);
  }
  .switch[aria-checked="true"] .knob {
    transform: translateX(14px);
    background: var(--lumen);
  }
  .resources {
    display: grid;
    gap: var(--s-3);
  }
  .resources h2 {
    font-family: var(--font-ui);
  }
  .resources ul {
    border-top: 1px solid var(--line);
  }
  .resources button {
    width: 100%;
    display: grid;
    grid-template-columns: 20px minmax(0, 1fr) auto;
    align-items: center;
    gap: var(--s-3);
    padding: var(--s-3) var(--s-2);
    border-bottom: 1px solid var(--line);
    text-align: left;
    font-size: var(--fs-sm);
    color: var(--text);
  }
  .resources button:hover {
    background: var(--surface-1);
  }
  .resources button :global(svg) {
    color: var(--moon);
  }
  .resources button[aria-pressed="true"] {
    color: var(--lumen);
  }
  .badge {
    margin-left: var(--s-2);
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.08em;
    color: var(--text-muted);
    border: 1px solid var(--line);
    border-radius: var(--r-1);
    padding: 0 var(--s-1);
    vertical-align: 1px;
  }
  .rname {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .rkind {
    font-size: var(--fs-xs);
    color: var(--text-muted);
  }
  .shortcuts {
    font-size: var(--fs-xs);
    color: var(--text-muted);
    line-height: 1.8;
  }
  .shortcuts .eyebrow {
    margin-right: var(--s-2);
  }
  .side {
    position: sticky;
    top: 0;
    height: 100vh;
    overflow-y: auto;
    border-left: 1px solid var(--line);
    background: var(--surface-1);
  }
  .close {
    display: none;
  }
  .theatre {
    grid-template-columns: minmax(0, 1fr);
  }
  .theatre .side {
    display: none;
  }
  .theatre .info {
    margin-inline: auto;
  }

  @media (max-width: 1100px) {
    .player {
      grid-template-columns: minmax(0, 1fr);
    }
    .drawer-btn {
      display: inline-flex;
    }
    .side,
    .theatre .side {
      display: block;
      position: fixed;
      inset: 0 0 0 auto;
      width: min(380px, 88vw);
      z-index: 10;
      transform: translateX(100%);
      visibility: hidden;
      transition:
        transform var(--dur-3) var(--ease-out),
        visibility 0s linear var(--dur-3);
    }
    .side.open {
      transform: none;
      visibility: visible;
      transition: transform var(--dur-3) var(--ease-out);
    }
    .close {
      display: inline-grid;
      position: absolute;
      top: var(--s-3);
      right: var(--s-3);
    }
    .info {
      padding-inline: var(--s-6);
    }
  }
  .headrow {
    display: grid;
    gap: var(--s-6);
  }

  /* Short windows: video capped, one-line title and controls stay in view. */
  @media (max-height: 640px) {
    .topbar {
      height: 44px;
    }
    .novideo {
      width: min(100%, calc(52vh * 16 / 9));
    }
    .info {
      gap: var(--s-6);
      padding-top: var(--s-3);
    }
    .headrow {
      display: flex;
      align-items: center;
      gap: var(--s-4);
      padding-bottom: var(--s-3);
      border-bottom: 1px solid var(--line);
    }
    .headrow header {
      flex: 1;
      min-width: 0;
      gap: 2px;
    }
    .headrow .eyebrow {
      font-size: 10px;
    }
    .headrow h1 {
      font-size: var(--fs-lg);
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
    }
    .actions {
      flex-wrap: nowrap;
      padding-bottom: 0;
      border-bottom: 0;
      gap: var(--s-1);
    }
    .actions .lbl {
      display: none;
    }
    .actions .btn {
      height: 36px;
      padding: 0 var(--s-3);
    }
    .actions .spacer {
      display: none;
    }
    .switch {
      margin-left: var(--s-2);
    }
  }

  @media (max-width: 760px) {
    .crumb {
      display: none;
    }
    .info {
      padding: var(--s-6) var(--s-4) var(--s-16);
    }
  }
</style>
