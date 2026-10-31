<script lang="ts">
  import { displayTitle } from "../../lib/logic/title";
  // Lessons of the current course, grouped by module; the current one glows.
  import { tick } from "svelte";
  import Icon from "../../lib/design/Icon.svelte";
  import { t } from "../../lib/i18n/index.svelte";
  import { courseStats, sectionKey, sectionNumber } from "../../lib/logic/progress";
  import ItemMenu from "../library/ItemMenu.svelte";
  import { deletion } from "../../lib/state/deletion.svelte";
  import { wording } from "../../lib/logic/videos";
  import { app } from "../../lib/state/app.svelte";
  import { href } from "../../lib/state/router.svelte";
  import type { LessonVideo } from "../../lib/logic/lessonVideos";
  import type { Course } from "../../lib/types";

  interface Props {
    group: string;
    course: Course;
    currentKey: string;
    watched: ReadonlySet<string>;
    onpick?: () => void;
    /** Videos of the lesson being played (a workout: one per exercise); listed under it. */
    videos?: LessonVideo[];
    videoIndex?: number;
    onvideo?: (index: number) => void;
  }
  let { group, course, currentKey, watched, onpick, videos = [], videoIndex = 0, onvideo }: Props = $props();
  let list: HTMLElement | undefined = $state();
  /** Whether the playing lesson shows its videos; one choice for every lesson, remembered. */
  let videosOpen = $state(readOpen());
  const uid = $props.id();

  function readOpen(): boolean {
    try {
      return localStorage.getItem("lumen.videosOpen") !== "0";
    } catch {
      return true;
    }
  }

  function toggleVideos() {
    videosOpen = !videosOpen;
    try {
      localStorage.setItem("lumen.videosOpen", videosOpen ? "1" : "0");
    } catch {
      /* not persisted */
    }
  }
  const stats = $derived(courseStats(course, watched));
  const g = $derived(app.group(group));
  const wd = $derived(wording(g?.platform ?? ""));
  const loose = $derived(g?.collection === "standalone");

  $effect(() => {
    void currentKey;
    void tick().then(() => {
      // Scroll only the sidebar, never the page, to center the current lesson.
      const el = list?.querySelector<HTMLElement>('[aria-current="true"]');
      const box = list?.parentElement;
      if (el && box) box.scrollTop = Math.max(0, el.offsetTop - box.clientHeight / 2);
    });
  });
</script>

<div class="lessons" bind:this={list}>
  <header>
    <span class="eyebrow">{t(wd.title)}</span>
    <a
      class="course"
      href={loose ? href({ name: "library", platform: "youtube" }) : href({ name: "course", group, course: course.index })}
    >{loose ? t("videos_title") : displayTitle(course.title)}</a>
    <span class="count num">{t(wd.countOf, { done: stats.done, total: stats.total })} · {stats.pct}%</span>
  </header>
  {#each course.sections as section, position (sectionKey(section))}
    <section aria-label={displayTitle(section.title)}>
      {#if course.sections.length > 1}
        <h3><span class="num idx">{String(sectionNumber(section, position)).padStart(2, "0")}</span>{displayTitle(section.title)}</h3>
      {/if}
      <ol>
        {#each section.lessons as lesson (lesson.key)}
          {@const done = watched.has(lesson.key)}
          {@const current = lesson.key === currentKey}
          {@const foldable = current && videos.length > 1}
          <li class="lrow">
            <a
              href={href({ name: "watch", group, key: lesson.key })}
              aria-current={current ? "true" : undefined}
              class:done
              class:novideo={!lesson.hasVideo}
              class:foldable
              onclick={onpick}
            >
              <span class="mark" aria-hidden="true">
                <Icon name={done ? "check" : lesson.hasVideo ? "play" : "noVideo"} size={14} />
              </span>
              <span class="title">{displayTitle(lesson.title)}</span>
              {#if foldable && !videosOpen}
                <span class="vcount num">{videoIndex + 1}/{videos.length}</span>
              {/if}
              <span class="visually-hidden">
                {current ? t("current") : ""}
                {done ? t("watched") : lesson.hasVideo ? t("not_watched") : t("no_video")}
              </span>
            </a>
            {#if foldable}
              <button
                type="button"
                class="fold"
                aria-expanded={videosOpen}
                aria-controls="videos-{uid}"
                aria-label={t(videosOpen ? "videos_hide" : "videos_show")}
                title={t(videosOpen ? "videos_hide" : "videos_show")}
                onclick={toggleVideos}
              >
                <Icon name={videosOpen ? "chevronDown" : "chevronRight"} size={16} />
              </button>
            {/if}
            <span class="menu-slot">
              <ItemMenu video label={displayTitle(lesson.title)} ondelete={() => deletion.openLesson(group, lesson.key)} />
            </span>
          </li>
          {#if current && videos.length > 1 && videosOpen}
            <li class="vlist">
              <ol id="videos-{uid}" aria-label={t("video_list")}>
                {#each videos as video, i (video.path)}
                  <li>
                    <button type="button" aria-current={i === videoIndex ? "true" : undefined} onclick={() => onvideo?.(i)}>
                      <span class="vnum num" aria-hidden="true">{i + 1}</span>
                      <span class="vtitle">{video.title}</span>
                    </button>
                  </li>
                {/each}
              </ol>
            </li>
          {/if}
        {/each}
      </ol>
    </section>
  {/each}
</div>

<style>
  .lessons {
    padding: var(--s-6) var(--s-3) var(--s-12);
  }
  header {
    display: grid;
    gap: var(--s-1);
    padding: 0 var(--s-3) var(--s-5);
    border-bottom: 1px solid var(--line);
    margin-bottom: var(--s-3);
  }
  .course {
    font-family: var(--font-display);
    font-weight: var(--fw-display);
    letter-spacing: var(--tracking-title);
    font-size: var(--fs-lg);
    line-height: var(--lh-snug);
    color: var(--text);
    text-decoration: none;
  }
  .course:hover {
    color: var(--lumen-hover);
  }
  .count {
    font-size: var(--fs-xs);
    color: var(--text-muted);
  }
  h3 {
    display: flex;
    gap: var(--s-2);
    font-family: var(--font-ui);
    font-size: var(--fs-2xs);
    font-weight: 500;
    letter-spacing: var(--tracking-label);
    text-transform: uppercase;
    color: var(--text-muted);
    padding: var(--s-5) var(--s-3) var(--s-2);
    line-height: 1.4;
  }
  .idx {
    color: var(--text-faint);
  }
  li a {
    position: relative;
    display: grid;
    grid-template-columns: 18px minmax(0, 1fr);
    gap: var(--s-3);
    align-items: start;
    padding: var(--s-2) var(--s-3);
    border-radius: var(--r-2);
    color: var(--text);
    text-decoration: none;
    font-size: var(--fs-sm);
    line-height: 1.4;
  }
  li a:hover {
    background: var(--surface-2);
  }
  .mark {
    color: var(--text-muted);
    padding-top: 1px;
  }
  .done .mark {
    color: var(--ok);
  }
  .novideo .title {
    color: var(--text-muted);
  }
  a[aria-current="true"] {
    background: radial-gradient(120% 140% at 0% 50%, var(--lumen-glow), transparent 70%);
  }
  a[aria-current="true"]::before {
    content: "";
    position: absolute;
    left: 0;
    top: 8px;
    bottom: 8px;
    width: 2px;
    background: var(--lumen);
  }
  a[aria-current="true"] .mark {
    color: var(--lumen);
  }
  .lrow {
    position: relative;
  }
  .lrow > a {
    padding-right: 44px;
  }
  .lrow > a.foldable {
    padding-right: 84px;
  }
  .fold {
    position: absolute;
    right: calc(var(--s-1) + 32px + var(--s-1));
    top: 50%;
    transform: translateY(-50%);
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border-radius: var(--r-2);
    color: var(--text-muted);
  }
  .fold:hover,
  .fold:focus-visible {
    background: var(--surface-2);
    color: var(--text);
  }
  .vcount {
    grid-column: 2;
    font-size: var(--fs-xs);
    color: var(--lumen);
  }
  .menu-slot {
    position: absolute;
    right: var(--s-1);
    top: 50%;
    transform: translateY(-50%);
  }
  /* An open menu must paint above the rows that follow it. */
  .menu-slot:focus-within {
    z-index: 10;
  }
  .vlist ol {
    display: grid;
    gap: 1px;
    padding: 0 var(--s-3) var(--s-2) calc(var(--s-3) + 18px + var(--s-3));
  }
  .vlist button {
    display: grid;
    grid-template-columns: 22px minmax(0, 1fr);
    gap: var(--s-2);
    align-items: start;
    width: 100%;
    padding: var(--s-1) var(--s-2);
    border-radius: var(--r-1);
    font-size: var(--fs-xs);
    line-height: 1.4;
    text-align: left;
    color: var(--text-muted);
  }
  .vlist button:hover,
  .vlist button:focus-visible {
    background: var(--surface-2);
    color: var(--text);
  }
  .vlist button[aria-current="true"] {
    color: var(--lumen);
  }
  .vnum {
    color: var(--text-faint);
  }
  .vlist button[aria-current="true"] .vnum {
    color: var(--lumen);
  }
</style>
