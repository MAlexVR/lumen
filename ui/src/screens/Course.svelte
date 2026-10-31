<script lang="ts">
  import { displayTitle } from "../lib/logic/title";
  import Icon from "../lib/design/Icon.svelte";
  import PosterArt from "../lib/components/PosterArt.svelte";
  import CardPreview from "../lib/components/CardPreview.svelte";
  import { cardPreviewFor } from "../lib/logic/cardPreview";
  import ProgressLine from "../lib/components/ProgressLine.svelte";
  import DeleteDialog from "./library/DeleteDialog.svelte";
  import Playlist from "./Playlist.svelte";
  import ModuleRow from "./course/ModuleRow.svelte";
  import { t, tn } from "../lib/i18n/index.svelte";
  import { courseStats, flattenCourse, nextUnwatched } from "../lib/logic/progress";
  import { app } from "../lib/state/app.svelte";
  import { href } from "../lib/state/router.svelte";

  let { group, course: courseIndex }: { group: string; course: number } = $props();

  const g = $derived(app.group(group));
  const course = $derived(g?.courses.find((c) => c.index === courseIndex));
  const watched = $derived(app.watchedOf(group));
  const stats = $derived(course ? courseStats(course, watched) : null);
  const target = $derived(course ? nextUnwatched(flattenCourse(course), watched) : null);
  const preview = $derived(course ? cardPreviewFor(course, app.progressOf(group), app.mediaBase) : null);
  let hot = $state(false);
  const action = $derived(stats?.state === "done" ? "btn_review" : stats?.state === "new" ? "btn_start" : "btn_continue");
</script>

{#if g?.platform === "youtube"}
  <Playlist {group} course={courseIndex} />
{:else if course && stats}
  <div class="page">
    <a class="back" href={href({ name: "library", platform: g?.platform ?? null })}>
      <Icon name="arrowLeft" size={16} />{g?.platform === "hotmart" ? "Hotmart" : g?.platform === "pluralsight" ? "Pluralsight" : g?.platform === "goldenlife" ? "Golden Life" : g?.platform === "harbiz" ? "Harbiz" : displayTitle(g?.title ?? group)}
    </a>

    <header class="head">
      <div class="art" role="presentation" onmouseenter={() => (hot = true)} onmouseleave={() => (hot = false)}>
        <PosterArt title={course.title} index={course.index} />
        {#if preview}
          <CardPreview src={preview.src} path={preview.path} resumeAt={preview.resumeAt} {hot} index={course.index} width={640} />
        {/if}
      </div>
      <div class="text">
        <span class="eyebrow">
          {g?.platform === "hotmart"
            ? t("hotmart_course")
            : g?.platform === "pluralsight"
              ? t("ps_course")
              : g?.platform === "goldenlife"
                ? t("gl_course")
                : g?.platform === "harbiz"
                  ? t("hb_course")
                  : t("course_n", { n: String(course.index).padStart(2, "0") })}
        </span>
        <h1>{displayTitle(course.title)}</h1>
        {#if g?.seller}<p class="seller">{g.seller}</p>{/if}
        <p class="meta num">
          <span>{tn("modules", course.sections.length)}</span>
          <span aria-hidden="true">·</span>
          <span>{tn("lessons", stats.total)}</span>
          {#if stats.noVideo}
            <span aria-hidden="true">·</span>
            <span>{t("novideo_n", { n: stats.noVideo })}</span>
          {/if}
        </p>
        <div class="progress">
          <div class="figure">
            <span class="big num">{stats.pct}<span class="pct">%</span></span>
            <span class="state" data-state={stats.state}>
              {#if stats.state === "done"}<Icon name="check" size={16} />{/if}
              {t(`state_${stats.state}` as const)}
            </span>
            <span class="count num">{t("lessons_count", { done: stats.done, total: stats.total })}</span>
          </div>
          <ProgressLine value={stats.pct} label={displayTitle(course.title)} />
        </div>
        {#if target}
          <div class="actions">
            <a class="btn btn-primary" href={href({ name: "watch", group, key: target.lesson.key })}>
              <Icon name="play" size={16} />{t(action)}
            </a>
            {#if stats.state === "progress"}
              <span class="next"><span class="eyebrow">{t("next")}</span> {displayTitle(target.lesson.title)}</span>
            {/if}
          </div>
        {/if}
      </div>
    </header>

    <ol class="modules">
      {#each course.sections as section (section.index)}
        <ModuleRow {group} {section} {watched} defaultOpen={course.sections.length === 1} />
      {/each}
    </ol>
  </div>
  <DeleteDialog onclosed={() => undefined} />
{:else}
  <div class="gone" role="status">
    <p>{t("gone_course")}</p>
    <a class="btn btn-quiet" href={href({ name: "library", platform: null })}>{t("gone_back")}</a>
  </div>
{/if}

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

  .page {
    padding: var(--s-8) var(--gutter) var(--s-20);
    max-width: var(--content-max);
  }
  .back {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    color: var(--text-muted);
    text-decoration: none;
    font-size: var(--fs-sm);
  }
  .back:hover {
    color: var(--text);
  }
  .head {
    position: relative;
    display: grid;
    grid-template-columns: minmax(160px, 240px) minmax(0, 1fr);
    gap: var(--s-12);
    align-items: end;
    padding: var(--s-10) 0 var(--s-12);
  }
  .art {
    position: relative;
    aspect-ratio: 2 / 3;
    border-radius: var(--r-2);
    overflow: hidden;
    outline: 1px solid var(--line);
    outline-offset: -1px;
  }
  .text {
    display: grid;
    gap: var(--s-4);
    min-width: 0;
  }
  h1 {
    font-size: var(--fs-display);
    line-height: 1.04;
  }
  .seller {
    color: var(--text-muted);
    font-size: var(--fs-md);
    margin-top: calc(-1 * var(--s-2));
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-2);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .progress {
    display: grid;
    gap: var(--s-3);
    max-width: 560px;
    margin-top: var(--s-2);
  }
  .figure {
    display: flex;
    align-items: baseline;
    gap: var(--s-4);
  }
  .big {
    font-family: var(--font-display);
    font-size: var(--fs-xl);
    font-weight: var(--fw-numeral);
    letter-spacing: -0.035em;
    line-height: 1;
  }
  .pct {
    font-size: 0.55em;
    color: var(--text-muted);
  }
  .state {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    font-size: var(--fs-sm);
    color: var(--text-muted);
  }
  .state[data-state="progress"] {
    color: var(--moon);
  }
  .state[data-state="done"] {
    color: var(--ok);
  }
  .count {
    margin-left: auto;
    font-size: var(--fs-xs);
    color: var(--text-muted);
  }
  .actions {
    display: flex;
    align-items: center;
    gap: var(--s-4);
    margin-top: var(--s-2);
    min-width: 0;
  }
  .actions a {
    text-decoration: none;
  }
  .next {
    color: var(--text-muted);
    font-size: var(--fs-sm);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .modules {
    border-top: 1px solid var(--line);
  }
  @media (max-width: 900px) {
    .head {
      grid-template-columns: 120px minmax(0, 1fr);
      gap: var(--s-6);
      align-items: start;
    }
    h1 {
      font-size: var(--fs-xl);
    }
  }
</style>
