<script lang="ts">
  // One labelled row of platform tiles; arrow keys move between tiles.
  import { tick } from "svelte";
  import { t } from "../../lib/i18n/index.svelte";
  import { gridMove, PLATFORMS, type PlatformStatus } from "../../lib/platforms";
  import { href } from "../../lib/state/router.svelte";
  import PlatformStage from "./PlatformStage.svelte";

  let { status: shown, focusId = null }: { status: PlatformStatus; focusId?: string | null } = $props();
  let list: HTMLElement | undefined = $state();
  const uid = $props.id();
  const ordered = $derived(PLATFORMS.filter((p) => p.status === shown));

  // Coming back from a platform puts focus on its tile.
  $effect(() => {
    if (!focusId) return;
    void tick().then(() => list?.querySelector<HTMLElement>(`[data-id="${focusId}"]`)?.focus());
  });

  function columns(tiles: HTMLElement[]): number {
    const top = tiles[0]?.offsetTop;
    const n = tiles.filter((el) => el.offsetTop === top).length;
    return Math.max(1, n);
  }

  function onKey(e: KeyboardEvent) {
    const tiles = [...(list?.querySelectorAll<HTMLElement>("a.tile") ?? [])];
    const index = tiles.indexOf(e.target as HTMLElement);
    if (index < 0) return;
    const next = gridMove(index, e.key, columns(tiles), tiles.length);
    if (next === null || next === index) return;
    e.preventDefault();
    tiles[next].focus();
  }
</script>

<section class="row" aria-labelledby="pf-row-{uid}">
<h2 id="pf-row-{uid}" class="eyebrow">{t(shown === "available" ? "pf_group_available" : "pf_group_soon")}</h2>
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<ul class="grid" bind:this={list} aria-labelledby="pf-row-{uid}" onkeydown={onKey}>
  {#each ordered as platform (platform.id)}
    {@const status = t(platform.status === "available" ? "pf_available" : "pf_soon")}
    <li>
      <a
        class="tile"
        href={href(platform.route)}
        data-id={platform.id}
        data-status={platform.status}
        aria-label="{platform.name}, {status}{platform.id === 'udemy' ? `, ${t('ud_tile_note')}` : ''}"
      >
        <PlatformStage {platform} />
        <span class="meta">
          <span class="name">{platform.name}</span>
          <span class="status"><span class="marker" aria-hidden="true"></span>{status}</span>
          {#if platform.id === "udemy"}<span class="tnote">{t("ud_tile_note")}</span>{/if}
        </span>
      </a>
    </li>
  {/each}
</ul>
</section>

<style>
  .row {
    display: grid;
    gap: var(--s-4);
  }
  .row h2 {
    font-family: var(--font-ui);
  }
  /* Rows of three: available platforms fill one, coming-soon ones start the
     next row under their own heading. */
  .grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: var(--s-6) var(--s-5);
    max-width: clamp(560px, calc((100vh - 230px) * 2.2), 1000px);
  }
  @media (max-width: 520px) {
    .grid {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
  }
  .tile {
    display: grid;
    gap: var(--s-3);
    color: inherit;
    text-decoration: none;
    border-radius: var(--r-2);
  }
  .tile :global(.stage) {
    outline: 1px solid var(--line);
    outline-offset: 3px;
    transition:
      outline-color var(--dur-2) var(--ease-out),
      box-shadow var(--dur-3) var(--ease-out),
      opacity var(--dur-2) var(--ease-out);
  }
  .tile:focus-visible {
    outline: none;
  }
  .tile[data-status="available"]:hover :global(.stage),
  .tile[data-status="available"]:focus-visible :global(.stage) {
    outline-color: var(--lumen);
    box-shadow: 0 0 48px 6px var(--lumen-glow);
  }
  .tile[data-status="soon"] :global(.stage) {
    opacity: 0.86;
  }
  .tile[data-status="soon"]:hover :global(.stage),
  .tile[data-status="soon"]:focus-visible :global(.stage) {
    opacity: 1;
    outline-color: var(--text-muted);
  }
  .tile[data-status="soon"]:focus-visible :global(.stage) {
    outline: var(--focus-ring);
  }
  .meta {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--s-1);
    min-width: 0;
  }
  .name {
    font-family: var(--font-display);
    font-size: 1.125rem;
    font-weight: var(--fw-display);
    letter-spacing: var(--tracking-title);
    line-height: var(--lh-snug);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  [data-status="soon"] .name {
    color: var(--text-muted);
  }
  .tnote {
    font-size: var(--fs-xs);
    color: var(--text-muted);
  }
  .status {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    flex: none;
    font-size: var(--fs-2xs);
    font-weight: 500;
    letter-spacing: var(--tracking-label);
    text-transform: uppercase;
    color: var(--text-muted);
  }
  [data-status="available"] .status {
    color: var(--lumen);
  }
  .marker {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    border: 1px solid var(--text-muted);
  }
  [data-status="available"] .marker {
    background: var(--lumen);
    border-color: var(--lumen);
  }
</style>
