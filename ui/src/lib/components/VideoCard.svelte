<script lang="ts">
  // A 16:9 card for a YouTube video or playlist (YouTube is not a course
  // platform). Same thumbnail/hover-preview mechanism as the poster cards.
  import { displayTitle } from "../logic/title";
  import CardPreview from "./CardPreview.svelte";
  import PosterArt from "./PosterArt.svelte";
  import ProgressLine from "./ProgressLine.svelte";

  interface Props {
    title: string;
    index: number;
    href: string;
    /** Watched share, 0-100. */
    pct: number;
    /** Hint under the title (watched state or "N videos"). */
    hint: string;
    hintState?: "done" | "progress" | "new";
    /** Playlists get a stacked "queue" look. */
    stack?: boolean;
    preview?: { src: string | null; path: string | null; resumeAt: number } | null;
    /** The video's own thumbnail (media server URL). */
    still?: string | null;
  }
  let { title, index, href, pct, hint, hintState = "new", stack = false, preview = null, still = null }: Props = $props();
  let hot = $state(false);
</script>

<a
  class="vcard"
  class:stack
  {href}
  onmouseenter={() => (hot = true)}
  onmouseleave={() => (hot = false)}
  onfocus={() => (hot = true)}
  onblur={() => (hot = false)}
>
  <div class="art">
    <PosterArt {title} {index} shape="still" />
    {#if preview || still}
      <CardPreview src={preview?.src ?? null} path={preview?.path ?? null} {still} resumeAt={preview?.resumeAt ?? 0} {hot} {index} />
    {/if}
    <div class="progress"><ProgressLine value={pct} label={displayTitle(title)} /></div>
  </div>
  <div class="meta">
    <h3 class="title">{displayTitle(title)}</h3>
    <p class="hint" data-state={hintState}>{hint}</p>
  </div>
</a>

<style>
  .vcard {
    display: grid;
    gap: var(--s-3);
    color: inherit;
    text-decoration: none;
    position: relative;
  }
  .art {
    position: relative;
    aspect-ratio: 16 / 9;
    border-radius: var(--r-2);
    overflow: hidden;
    outline: 1px solid var(--line-soft);
    outline-offset: -1px;
    transition: filter var(--dur-2) var(--ease-out);
  }
  /* Queue look: two hairline edges peeking out above the card. */
  .stack {
    padding-top: 10px;
  }
  .stack::before,
  .stack::after {
    content: "";
    position: absolute;
    top: 0;
    height: 5px;
    border: 1px solid var(--line);
    border-bottom: none;
    border-radius: var(--r-2) var(--r-2) 0 0;
    background: var(--surface-1);
  }
  .stack::before {
    left: 12px;
    right: 12px;
  }
  .stack::after {
    left: 6px;
    right: 6px;
    top: 5px;
    background: var(--surface-2);
  }
  .progress {
    z-index: 3;
    position: absolute;
    inset: auto 0 0 0;
  }
  .vcard:hover .art {
    filter: brightness(1.25);
  }
  .vcard:focus-visible {
    outline: none;
  }
  .vcard:focus-visible .art {
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
  .hint {
    font-size: var(--fs-xs);
    color: var(--text-muted);
  }
  .hint[data-state="done"] {
    color: var(--ok);
  }
  .hint[data-state="progress"] {
    color: var(--moon);
  }
</style>
