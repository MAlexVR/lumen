<script lang="ts">
  import { displayTitle } from "../logic/title";
  import { t } from "../i18n/index.svelte";
  import type { Stats } from "../logic/progress";
  import CardPreview from "./CardPreview.svelte";
  import PosterArt from "./PosterArt.svelte";
  import ProgressLine from "./ProgressLine.svelte";

  interface Props {
    title: string;
    index: number;
    stats: Stats;
    href: string;
    eyebrow?: string;
    /** Secondary line under the title (Hotmart seller). */
    subtitle?: string | null;
    /** Real video still/preview of the course (logic/cardPreview.ts); poster otherwise. */
    preview?: { src: string; path: string; resumeAt: number } | null;
  }
  let { title, index, stats, href, eyebrow, subtitle = null, preview = null }: Props = $props();
  let hot = $state(false);
  const stateLabel = $derived(t(`state_${stats.state}` as const));
</script>

<a
  class="poster"
  {href}
  onmouseenter={() => (hot = true)}
  onmouseleave={() => (hot = false)}
  onfocus={() => (hot = true)}
  onblur={() => (hot = false)}
>
  <div class="art">
    <PosterArt {title} {index} />
    {#if preview}<CardPreview src={preview.src} path={preview.path} resumeAt={preview.resumeAt} {hot} {index} />{/if}
    <div class="progress"><ProgressLine value={stats.pct} label={displayTitle(title)} /></div>
  </div>
  <div class="meta">
    {#if eyebrow}<span class="eyebrow">{eyebrow}</span>{/if}
    <h3 class="title">{displayTitle(title)}</h3>
    {#if subtitle}<p class="by">{subtitle}</p>{/if}
    <p class="sub">
      <span class="state" data-state={stats.state}>{stateLabel}</span>
      <span class="dot" aria-hidden="true">·</span>
      <span class="num">{t("lessons_count", { done: stats.done, total: stats.total })}</span>
    </p>
  </div>
</a>

<style>
  .poster {
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
    color: inherit;
    text-decoration: none;
    min-width: 0;
  }
  .art {
    position: relative;
    aspect-ratio: 2 / 3;
    border-radius: var(--r-2);
    overflow: hidden;
    outline: 1px solid var(--line-soft);
    outline-offset: -1px;
    transition:
      filter var(--dur-2) var(--ease-out),
      transform var(--dur-3) var(--ease-out);
  }
  .progress {
    z-index: 3;
    position: absolute;
    inset: auto 0 0 0;
  }
  .poster:hover .art {
    filter: brightness(1.35);
  }
  .poster:focus-visible {
    outline: none;
  }
  .poster:focus-visible .art {
    outline: var(--focus-ring);
    outline-offset: var(--focus-offset);
  }
  .meta {
    display: grid;
    gap: var(--s-1);
  }
  .title {
    line-height: var(--lh-snug);
    font-size: 1rem;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .by {
    font-size: var(--fs-xs);
    color: var(--text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .sub {
    font-size: var(--fs-xs);
    color: var(--text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .sub > span + span {
    margin-left: var(--s-1);
  }
  .state[data-state="done"] {
    color: var(--ok);
  }
  .state[data-state="progress"] {
    color: var(--moon);
  }
  .dot {
    color: var(--text-faint);
  }
</style>
