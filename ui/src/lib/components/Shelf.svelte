<script lang="ts">
  // Horizontal row of posters with an editorial heading and paging buttons.
  import type { Snippet } from "svelte";
  import Icon from "../design/Icon.svelte";
  import { t } from "../i18n/index.svelte";

  interface Props {
    title: string;
    meta?: string;
    href?: string;
    children: Snippet;
  }
  let { title, meta, href, children }: Props = $props();
  let row: HTMLElement | undefined = $state();
  const uid = $props.id();

  function page(dir: 1 | -1) {
    row?.scrollBy({ left: dir * row.clientWidth * 0.85, behavior: "smooth" });
  }
</script>

<section class="shelf" aria-labelledby="shelf-{uid}">
  <header>
    <h2 id="shelf-{uid}">
      {#if href}<a {href}>{title}</a>{:else}{title}{/if}
    </h2>
    {#if meta}<span class="meta num">{meta}</span>{/if}
    <span class="rule" aria-hidden="true"></span>
    <div class="paging">
      <button class="icon-btn" type="button" onclick={() => page(-1)} aria-label={t("prev")}>
        <Icon name="chevronRight" size={18} />
      </button>
      <button class="icon-btn" type="button" onclick={() => page(1)} aria-label={t("next")}>
        <Icon name="chevronRight" size={18} />
      </button>
    </div>
  </header>
  <ul class="row" bind:this={row}>
    {@render children()}
  </ul>
</section>

<style>
  .shelf {
    display: grid;
    gap: var(--s-5);
  }
  header {
    display: flex;
    align-items: baseline;
    gap: var(--s-4);
    padding: 0 var(--gutter);
  }
  h2 {
    font-size: var(--fs-lg);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
  h2 a {
    color: inherit;
    text-decoration: none;
  }
  h2 a:hover {
    color: var(--lumen-hover);
  }
  .meta {
    font-size: var(--fs-xs);
    color: var(--text-muted);
    white-space: nowrap;
  }
  .rule {
    flex: 1;
    height: 1px;
    background: var(--line);
    align-self: center;
  }
  .paging {
    display: flex;
    gap: var(--s-1);
    align-self: center;
  }
  .paging button:first-child :global(svg) {
    transform: scaleX(-1);
  }
  .row {
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: clamp(128px, 11.5vw, 172px);
    gap: var(--s-6);
    overflow-x: auto;
    scroll-snap-type: x mandatory;
    scroll-padding-inline: var(--gutter);
    padding: var(--s-1) var(--gutter) var(--s-4);
    scrollbar-width: none;
  }
  .row :global(> li) {
    scroll-snap-align: start;
  }
</style>
