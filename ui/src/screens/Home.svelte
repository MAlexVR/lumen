<script lang="ts">
  import Hero from "./home/Hero.svelte";
  import Poster from "../lib/components/Poster.svelte";
  import ProgressLine from "../lib/components/ProgressLine.svelte";
  import YtPlaylist from "../lib/components/YtPlaylist.svelte";
  import YtVideo from "../lib/components/YtVideo.svelte";
  import Shelf from "../lib/components/Shelf.svelte";
  import { t, tn } from "../lib/i18n/index.svelte";
  import { continueAcross, shelves } from "../lib/logic/library";
  import { cardPreviewFor } from "../lib/logic/cardPreview";
  import { isYoutube, wording, youtubeSections } from "../lib/logic/videos";
  import { courseStats, groupStats } from "../lib/logic/progress";
  import { displayTitle } from "../lib/logic/title";
  import { platformById } from "../lib/platforms";
  import { app } from "../lib/state/app.svelte";
  import { href } from "../lib/state/router.svelte";
  import type { Course, Group } from "../lib/types";

  // Continue spans platforms; progress stays per group (keyed by group.id).
  const current = $derived(continueAcross(app.library, (id) => app.progressOf(id), app.recentGroupId));
  const group = $derived(current?.group);
  const watched = $derived(group ? app.watchedOf(group.id) : new Set<string>());
  const overall = $derived(group ? groupStats(group, watched) : null);
  const rows = $derived(shelves(app.library).filter((r) => r.platform !== "youtube"));
  const yt = $derived(youtubeSections(app.groups, ""));
  const wd = $derived(wording(group?.platform ?? ""));

  const inProgress = $derived(
    app.groups.filter((g) => !isYoutube(g.platform)).flatMap((g) => {
      const w = app.watchedOf(g.id);
      return g.courses
        .map((c) => ({ group: g, course: c, stats: courseStats(c, w) }))
        .filter((x) => x.stats.state === "progress");
    }),
  );

  const courseHref = (g: Group, c: Course) => href({ name: "course", group: g.id, course: c.index });
  const previewOf = (g: Group, c: Course) => cardPreviewFor(c, app.progressOf(g.id), app.mediaBase);
</script>

<div class="home">
  {#if current}
    <Hero group={current.group} target={current.target} {watched} />
  {/if}

  {#if group && overall}
    <section class="overall" aria-labelledby="overall-h">
      <h2 id="overall-h" class="eyebrow">{t("overall")} · {displayTitle(group.title)}</h2>
      <div class="figure">
        <span class="big num">{overall.pct}<span class="pct">%</span></span>
        <p class="caption">
          <span class="num strong">{overall.done}</span>
          {t(wd.ofTotal, { total: overall.total })}
        </p>
      </div>
      <ProgressLine value={overall.pct} label={t("overall")} />
    </section>
  {/if}

  {#if inProgress.length}
    <Shelf title={t("shelf_in_progress")} meta={String(inProgress.length)}>
      {#each inProgress as x (x.group.id + x.course.index)}
        <li>
          <Poster
            title={x.course.title}
            index={x.course.index}
            stats={x.stats}
            eyebrow={x.group.platform === "skool" ? displayTitle(x.group.title) : platformById(x.group.platform)?.name}
            subtitle={x.group.platform === "hotmart" ? x.group.seller : null}
            href={courseHref(x.group, x.course)}
            preview={previewOf(x.group, x.course)}
          />
        </li>
      {/each}
    </Shelf>
  {/if}

  {#each rows as row (row.key)}
    <Shelf
      title={row.platform === "skool" ? displayTitle(row.title) : (platformById(row.platform)?.name ?? row.platform)}
      meta={tn("courses", row.items.length)}
      href={href({ name: "library", platform: row.platform })}
    >
      {#each row.items as item (item.group.id + item.course.index)}
        <li>
          <Poster
            title={item.course.title}
            index={row.platform === "skool" ? item.course.index : row.items.indexOf(item) + 1}
            stats={courseStats(item.course, app.watchedOf(item.group.id))}
            subtitle={item.group.seller}
            href={courseHref(item.group, item.course)}
            preview={previewOf(item.group, item.course)}
          />
        </li>
      {/each}
    </Shelf>
  {/each}

  {#if yt.videos.length}
    <Shelf title={t("yt_shelf_videos")} meta={tn("videos", yt.videos.length)} href={href({ name: "library", platform: "youtube" })} wide>
      {#each yt.videos as v, n (v.lesson.key)}
        <li><YtVideo item={v} index={n + 1} /></li>
      {/each}
    </Shelf>
  {/if}
  {#if yt.playlists.length}
    <Shelf title={t("yt_shelf_playlists")} meta={String(yt.playlists.length)} href={href({ name: "library", platform: "youtube" })} wide>
      {#each yt.playlists as g, n (g.id)}
        <li><YtPlaylist group={g} index={n + 1} /></li>
      {/each}
    </Shelf>
  {/if}
</div>

<style>
  .home {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: var(--s-16);
    padding-bottom: var(--s-20);
  }
  .overall {
    display: grid;
    grid-template-columns: auto 1fr;
    align-items: end;
    gap: var(--s-3) var(--s-10);
    padding: 0 var(--gutter);
    margin-top: calc(-1 * var(--s-6));
  }
  .overall h2 {
    grid-column: 1 / -1;
    font-family: var(--font-ui);
  }
  .figure {
    display: flex;
    align-items: baseline;
    gap: var(--s-5);
  }
  .big {
    font-family: var(--font-display);
    font-size: var(--fs-2xl);
    font-weight: var(--fw-numeral);
    letter-spacing: -0.04em;
    line-height: 0.9;
  }
  .pct {
    font-size: 0.5em;
    color: var(--text-muted);
    margin-left: 2px;
  }
  .caption {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .strong {
    color: var(--text);
  }
  .overall :global([role="progressbar"]) {
    align-self: center;
    margin-bottom: var(--s-2);
  }
</style>
