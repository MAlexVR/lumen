<script lang="ts">
  import Icon from "../lib/design/Icon.svelte";
  import PlatformChip from "../lib/components/PlatformChip.svelte";
  import Poster from "../lib/components/Poster.svelte";
  import { t, tn } from "../lib/i18n/index.svelte";
  import { libraryEntries, platformsPresent, type LibraryEntry } from "../lib/logic/library";
  import { cardPreviewFor } from "../lib/logic/cardPreview";
  import { youtubeSections } from "../lib/logic/videos";
  import YtPlaylist from "../lib/components/YtPlaylist.svelte";
  import YtVideo from "../lib/components/YtVideo.svelte";
  import { courseStats } from "../lib/logic/progress";
  import { displayTitle } from "../lib/logic/title";
  import { platformById } from "../lib/platforms";
  import { app } from "../lib/state/app.svelte";
  import { thumbs } from "../lib/state/thumbs.svelte";
  import { readDismissed, writeDismissed } from "../lib/logic/thumbs";
  import { href } from "../lib/state/router.svelte";
  import { deletion } from "../lib/state/deletion.svelte";
  import type { PlatformId } from "../lib/types";
  import DeleteDialog from "./library/DeleteDialog.svelte";
  import ItemMenu from "./library/ItemMenu.svelte";

  let { platform: filter }: { platform: string | null } = $props();
  let query = $state("");
  let heading: HTMLElement | undefined = $state();
  let noteDismissed = $state(readDismissed(localStorage));
  const dismissNote = () => {
    noteDismissed = true;
    writeDismissed(localStorage);
  };

  const present = $derived(platformsPresent(app.library));
  const active = $derived(filter && (present as string[]).includes(filter) ? filter : null);
  const entries = $derived(libraryEntries(app.library, active, query));
  const skoolEntries = $derived(entries.filter((e) => e.group.platform === "skool"));
  /** Platforms whose groups are one course each: one shelf of posters per platform. */
  const SINGLE: PlatformId[] = ["hotmart", "pluralsight", "goldenlife", "harbiz"];
  const singleSections = $derived(
    SINGLE.map((id) => ({ id, entries: entries.filter((e) => e.group.platform === id) })).filter((x) => x.entries.length),
  );
  // YouTube is not a course platform: videos and playlists.
  const yt = $derived(active && active !== "youtube" ? { videos: [], playlists: [] } : youtubeSections(app.groups, query));
  const courseCount = $derived(entries.filter((e) => e.group.platform !== "youtube").reduce((n, e) => n + e.courses.length, 0));
  const lessonCount = $derived(
    entries.filter((e) => e.group.platform !== "youtube").reduce((n, e) => n + e.courses.reduce((m, c) => m + c.sections.reduce((k, s) => k + s.lessons.length, 0), 0), 0),
  );
  const ytVideoCount = $derived(yt.videos.length + yt.playlists.reduce((n, g) => n + g.lessonCount, 0));
  const statsOf = (e: LibraryEntry, i: number) => courseStats(e.courses[i], app.watchedOf(e.group.id));
</script>

<div class="page">
  <header class="head">
    <div class="title">
      <span class="eyebrow">{t("nav_library")}</span>
      <h1 tabindex="-1" bind:this={heading}>{active ? platformById(active)?.name : t("nav_library")}</h1>
      <p class="meta num">
        {#if courseCount}
          <span>{tn("courses", courseCount)}</span>
          <span aria-hidden="true">·</span>
          <span>{tn("lessons", lessonCount)}</span>
        {/if}
        {#if ytVideoCount}
          {#if courseCount}<span aria-hidden="true">·</span>{/if}
          <span>{tn("videos", ytVideoCount)}</span>
        {/if}
      </p>
    </div>
    <label class="search">
      <span class="visually-hidden">{t("search_label")}</span>
      <Icon name="search" size={18} />
      <input type="search" bind:value={query} placeholder={t(active === "youtube" ? "search_placeholder_videos" : "search_placeholder")} autocomplete="off" spellcheck="false" />
    </label>
  </header>

  {#if present.length > 1}
    <nav class="tabs" aria-label={t("lib_platforms")}>
      <a href={href({ name: "library", platform: null })} aria-current={active === null ? "page" : undefined}>{t("lib_all")}</a>
      {#each present as id (id)}
        <a href={href({ name: "library", platform: id })} aria-current={active === id ? "page" : undefined}>
          <PlatformChip {id} />{platformById(id)?.name}
        </a>
      {/each}
    </nav>
  {/if}

  {#if deletion.notice}
    <p class="notice" role="status"><Icon name="check" size={16} />{deletion.notice}</p>
  {/if}

  {#if thumbs.ffmpegMissing && !noteDismissed}
    <div class="notice quiet" role="note">
      <Icon name="noVideo" size={16} />
      <p>{t("thumbs_need_ffmpeg")}</p>
      <button type="button" class="btn btn-quiet" onclick={dismissNote}>{t("thumbs_note_dismiss")}</button>
    </div>
  {/if}

  {#if !entries.length && !yt.videos.length && !yt.playlists.length}
    <p class="empty" role="status">{t("no_results", { q: query.trim() })}</p>
  {/if}

  {#each skoolEntries as entry (entry.group.id)}
    <section class="group" aria-labelledby="g-{entry.group.id}">
      <header class="ghead">
        <PlatformChip id="skool" />
        <h2 id="g-{entry.group.id}">{displayTitle(entry.group.title)}</h2>
        <span class="gmeta num">{t(`status_${entry.group.status}` as const)} · {tn("courses", entry.group.courses.length)}</span>
        <span class="gmenu">
          <ItemMenu label={displayTitle(entry.group.title)} ondelete={() => deletion.open(entry.group)} />
        </span>
      </header>
      <ul class="grid">
        {#each entry.courses as c, i (c.index)}
          <li class="card">
            <span class="card-menu">
              <ItemMenu label={displayTitle(c.title)} ondelete={() => deletion.open(entry.group, c)} />
            </span>
            <Poster
              title={c.title}
              index={c.index}
              stats={statsOf(entry, i)}
              href={href({ name: "course", group: entry.group.id, course: c.index })}
              preview={cardPreviewFor(c, app.progressOf(entry.group.id), app.mediaBase)}
            />
          </li>
        {/each}
      </ul>
    </section>
  {/each}

  {#each singleSections as section (section.id)}
    <section class="group" aria-labelledby="g-{section.id}">
      <header class="ghead">
        <PlatformChip id={section.id} />
        <h2 id="g-{section.id}">{platformById(section.id)?.name}</h2>
        <span class="gmeta num">{tn("courses", section.entries.length)}</span>
      </header>
      <ul class="grid">
        {#each section.entries as entry, n (entry.group.id)}
          <li class="card">
            <span class="card-menu">
              <ItemMenu label={displayTitle(entry.group.title)} ondelete={() => deletion.open(entry.group)} />
            </span>
            <Poster
              title={entry.courses[0].title}
              index={n + 1}
              stats={statsOf(entry, 0)}
              subtitle={entry.group.seller}
              href={href({ name: "course", group: entry.group.id, course: entry.courses[0].index })}
              preview={cardPreviewFor(entry.courses[0], app.progressOf(entry.group.id), app.mediaBase)}
            />
          </li>
        {/each}
      </ul>
    </section>
  {/each}

  {#if yt.videos.length || yt.playlists.length}
    <section class="group" aria-labelledby="g-youtube">
      <header class="ghead">
        <PlatformChip id="youtube" />
        <h2 id="g-youtube">{platformById("youtube")?.name}</h2>
        <span class="gmeta num">{tn("videos", ytVideoCount)}</span>
      </header>
      {#if yt.videos.length}
        <h3 class="sub">{t("yt_videos_section")}</h3>
        <ul class="vgrid">
          {#each yt.videos as v, n (v.lesson.key)}
            <li class="card">
              <span class="card-menu">
                <ItemMenu video label={displayTitle(v.lesson.title)} ondelete={() => deletion.openLesson(v.group.id, v.lesson.key)} />
              </span>
              <YtVideo item={v} index={n + 1} />
            </li>
          {/each}
        </ul>
      {/if}
      {#if yt.playlists.length}
        <h3 class="sub">{t("yt_playlists_section")}</h3>
        <ul class="vgrid">
          {#each yt.playlists as g, n (g.id)}
            <li class="card">
              <span class="card-menu">
                <ItemMenu label={displayTitle(g.title)} ondelete={() => deletion.open(g)} />
              </span>
              <YtPlaylist group={g} index={n + 1} />
            </li>
          {/each}
        </ul>
      {/if}
    </section>
  {/if}
</div>

<DeleteDialog onclosed={() => heading?.focus()} />

<style>
  .page {
    padding: var(--s-12) var(--gutter) var(--s-20);
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: var(--s-8);
  }
  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: end;
    justify-content: space-between;
    gap: var(--s-6);
  }
  .title {
    display: grid;
    gap: var(--s-2);
    min-width: 0;
  }
  h1 {
    font-size: var(--fs-2xl);
  }
  .meta {
    display: flex;
    gap: var(--s-2);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .search {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    width: min(340px, 100%);
    height: 40px;
    padding: 0 var(--s-3);
    border-bottom: 1px solid var(--line);
    color: var(--text-muted);
    transition: border-color var(--dur-2) var(--ease-out);
  }
  .search:focus-within {
    border-color: var(--lumen);
    color: var(--text);
  }
  .search input {
    flex: 1;
    min-width: 0;
    background: none;
    border: 0;
    outline: none;
    font-size: var(--fs-base);
  }
  .search input::placeholder {
    color: var(--text-muted);
  }
  .tabs {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-6);
    border-bottom: 1px solid var(--line);
  }
  .tabs a {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    padding: var(--s-2) 0 var(--s-3);
    color: var(--text-muted);
    text-decoration: none;
    font-size: var(--fs-sm);
    font-weight: 500;
  }
  .tabs a:hover,
  .tabs a[aria-current="page"] {
    color: var(--text);
  }
  .tabs a[aria-current="page"]::after {
    content: "";
    position: absolute;
    inset: auto 0 -1px 0;
    height: 1px;
    background: var(--lumen);
  }
  .sub {
    font-family: var(--font-ui);
    font-size: var(--fs-sm);
    font-weight: 500;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-muted);
  }
  .vgrid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
    gap: var(--s-8) var(--s-6);
  }
  .group {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: var(--s-5);
  }
  .ghead {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    min-width: 0;
  }
  .ghead h2 {
    font-size: var(--fs-lg);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .gmeta {
    font-size: var(--fs-xs);
    color: var(--text-muted);
    white-space: nowrap;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(144px, 1fr));
    gap: var(--s-10) var(--s-5);
    max-width: 1400px;
  }
  .notice {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    font-size: var(--fs-sm);
    color: var(--text);
    padding: var(--s-3) var(--s-4);
    background: var(--surface-1);
    border-left: 2px solid var(--ok);
    border-radius: 0 var(--r-2) var(--r-2) 0;
  }
  .notice :global(svg) {
    color: var(--ok);
  }
  .notice.quiet {
    border-left-color: var(--line);
  }
  .notice.quiet :global(svg) {
    color: var(--text-muted);
  }
  .notice.quiet p {
    flex: 1;
    margin: 0;
  }
  .gmenu {
    margin-left: auto;
  }
  .card {
    position: relative;
  }
  .card-menu {
    position: absolute;
    top: var(--s-2);
    right: var(--s-2);
    z-index: 2;
  }
  /* Discreet at rest, clear on hover or keyboard focus, always there on touch. */
  .card-menu :global(.kebab),
  .gmenu :global(.kebab) {
    opacity: 0.55;
  }
  .card:hover .card-menu :global(.kebab),
  .card:focus-within .card-menu :global(.kebab),
  .group:hover .gmenu :global(.kebab),
  .gmenu:focus-within :global(.kebab) {
    opacity: 1;
  }
  @media (hover: none) {
    .card-menu :global(.kebab),
    .gmenu :global(.kebab) {
      opacity: 1;
    }
  }
  .gmenu :global(.kebab) {
    background: transparent;
  }
  .empty {
    color: var(--text-muted);
    padding: var(--s-10) 0;
  }
</style>
