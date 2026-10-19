<script lang="ts">
  // A YouTube playlist: a flat numbered list of videos, no modules.
  import { mediaUrl } from "../lib/api";
  import CardPreview from "../lib/components/CardPreview.svelte";
  import PosterArt from "../lib/components/PosterArt.svelte";
  import ProgressLine from "../lib/components/ProgressLine.svelte";
  import Icon from "../lib/design/Icon.svelte";
  import { t, tn } from "../lib/i18n/index.svelte";
  import { flattenCourse, nextUnwatched } from "../lib/logic/progress";
  import { displayTitle } from "../lib/logic/title";
  import { previewSrc } from "../lib/logic/preview";
  import { app } from "../lib/state/app.svelte";
  import { href } from "../lib/state/router.svelte";

  let { group, course: courseIndex }: { group: string; course: number } = $props();

  const g = $derived(app.group(group));
  const course = $derived(g?.courses.find((c) => c.index === courseIndex));
  const watched = $derived(app.watchedOf(group));
  const flat = $derived(course ? flattenCourse(course) : []);
  const seen = $derived(flat.filter((f) => watched.has(f.lesson.key)).length);
  const pct = $derived(flat.length ? Math.round((seen / flat.length) * 100) : 0);
  const target = $derived(course ? nextUnwatched(flat, watched) : null);
  const last = $derived(app.progressOf(group).lastLesson);
  const action = $derived(pct >= 100 ? "btn_review" : seen > 0 || last ? "btn_continue" : "yt_play_list");
</script>

{#if g && course}
  <div class="page">
    <a class="back" href={href({ name: "library", platform: "youtube" })}>
      <Icon name="arrowLeft" size={16} />{t("yt_back")}
    </a>

    <header class="head">
      <span class="eyebrow">{t("yt_playlist_eyebrow")}</span>
      <h1>{displayTitle(g.title)}</h1>
      <p class="meta num">
        <span>{tn("videos", flat.length)}</span>
        <span aria-hidden="true">·</span>
        <span>{t("yt_seen_n", { n: seen })}</span>
      </p>
      <div class="bar"><ProgressLine value={pct} label={displayTitle(g.title)} /></div>
      {#if target}
        <div class="actions">
          <a class="btn btn-primary" href={href({ name: "watch", group, key: (last && flat.some((f) => f.lesson.key === last) ? last : target.lesson.key) })}>
            <Icon name="play" size={16} />{t(action)}
          </a>
        </div>
      {/if}
    </header>

    <ol class="videos">
      {#each flat as f, n (f.lesson.key)}
        {@const done = watched.has(f.lesson.key)}
        {@const l = f.lesson}
        <li>
          <a class="row" href={href({ name: "watch", group, key: l.key })} class:done>
            <span class="n num">{String(n + 1).padStart(2, "0")}</span>
            <span class="thumb">
              <PosterArt title={l.title} index={n + 1} shape="still" />
              <CardPreview
                src={null}
                path={l.video}
                still={l.thumbnail && app.mediaBase ? mediaUrl(app.mediaBase, l.thumbnail) : null}
                resumeAt={0}
                hot={false}
                index={n + 1}
                width={240}
              />
            </span>
            <span class="title">{displayTitle(l.title)}</span>
            <span class="state">
              <Icon name={done ? "check" : l.hasVideo ? "play" : "noVideo"} size={14} />
              <span>{done ? t("yt_video_seen") : l.hasVideo ? t("yt_video_new") : t("no_video")}</span>
            </span>
          </a>
        </li>
      {/each}
    </ol>
  </div>
{:else}
  <div class="gone" role="status">
    <p>{t("yt_playlist_gone")}</p>
    <a class="btn btn-quiet" href={href({ name: "library", platform: "youtube" })}>{t("gone_back")}</a>
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
  .gone a,
  .actions a {
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
    display: grid;
    gap: var(--s-4);
    padding: var(--s-10) 0 var(--s-10);
    max-width: 640px;
  }
  h1 {
    font-size: var(--fs-display);
    line-height: 1.04;
  }
  .meta {
    display: flex;
    gap: var(--s-2);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .actions {
    display: flex;
    margin-top: var(--s-2);
  }
  .videos {
    border-top: 1px solid var(--line);
  }
  .row {
    display: grid;
    grid-template-columns: 40px 128px minmax(0, 1fr) auto;
    align-items: center;
    gap: var(--s-4);
    padding: var(--s-3) var(--s-2);
    border-bottom: 1px solid var(--line);
    color: inherit;
    text-decoration: none;
    border-radius: var(--r-2);
  }
  .row:hover {
    background: var(--surface-1);
  }
  .row:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-offset);
  }
  .n {
    font-family: var(--font-display);
    font-size: var(--fs-lg);
    color: var(--text-muted);
  }
  .thumb {
    position: relative;
    aspect-ratio: 16 / 9;
    border-radius: var(--r-1);
    overflow: hidden;
    outline: 1px solid var(--line-soft);
    outline-offset: -1px;
  }
  .title {
    line-height: var(--lh-snug);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .state {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    font-size: var(--fs-xs);
    color: var(--text-muted);
  }
  .done .state {
    color: var(--ok);
  }
  @media (max-width: 760px) {
    .row {
      grid-template-columns: 28px 96px minmax(0, 1fr);
    }
    .state span {
      display: none;
    }
    .state {
      display: none;
    }
  }
</style>
