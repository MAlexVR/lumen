<script lang="ts">
  import { displayTitle } from "../../lib/logic/title";
  import Icon from "../../lib/design/Icon.svelte";
  import ProgressLine from "../../lib/components/ProgressLine.svelte";
  import ItemMenu from "../library/ItemMenu.svelte";
  import { deletion } from "../../lib/state/deletion.svelte";
  import { t } from "../../lib/i18n/index.svelte";
  import { flattenCourse, nextUnwatched, sectionStats } from "../../lib/logic/progress";
  import { href, navigate } from "../../lib/state/router.svelte";
  import type { Section } from "../../lib/types";

  interface Props {
    group: string;
    section: Section;
    watched: ReadonlySet<string>;
    defaultOpen?: boolean;
  }
  let { group, section, watched, defaultOpen = false }: Props = $props();
  // svelte-ignore state_referenced_locally
  let open = $state(defaultOpen);
  const uid = $props.id();

  const stats = $derived(sectionStats(section, watched));
  const info = $derived(
    [
      t("lessons_count", { done: stats.done, total: stats.total }),
      `${stats.pct}%`,
      t(`state_${stats.state}` as const),
      ...(stats.noVideo ? [t("novideo_n", { n: stats.noVideo })] : []),
    ].join(" · "),
  );

  function openNext() {
    const target = nextUnwatched(
      flattenCourse({ title: "", index: 0, sections: [section] }),
      watched,
    );
    if (target) navigate({ name: "watch", group, key: target.lesson.key });
  }
</script>

<li class="module" data-state={stats.state}>
  <div class="row">
    <button type="button" class="open" onclick={openNext} aria-label="{displayTitle(section.title)}, {info}">
      <span class="idx num" aria-hidden="true">{String(section.index).padStart(2, "0")}</span>
      <span class="body">
        <span class="title">{displayTitle(section.title)}</span>
        <span class="info num">
          {#if stats.state === "done"}<Icon name="check" size={14} />{/if}
          {info}
        </span>
      </span>
      <span class="bar"><ProgressLine value={stats.pct} label={displayTitle(section.title)} /></span>
    </button>
    <button
      type="button"
      class="toggle"
      aria-expanded={open}
      aria-controls="lessons-{uid}"
      onclick={() => (open = !open)}
    >
      <span class="toggle-label">{t(open ? "hide_lessons" : "show_lessons")}</span>
      <Icon name="chevronDown" size={16} />
    </button>
  </div>

  <ol class="lessons" id="lessons-{uid}" hidden={!open}>
    {#each section.lessons as lesson (lesson.key)}
      {@const done = watched.has(lesson.key)}
      <li class="lrow">
        <a href={href({ name: "watch", group, key: lesson.key })} class:done class:novideo={!lesson.hasVideo}>
          <span class="mark" aria-hidden="true">
            <Icon name={done ? "check" : lesson.hasVideo ? "play" : "noVideo"} size={14} />
          </span>
          <span class="ltitle">{displayTitle(lesson.title)}</span>
          <span class="lstate">
            {done ? t("watched") : lesson.hasVideo ? t("not_watched") : t("no_video")}
          </span>
        </a>
        <span class="menu-slot">
          <ItemMenu video label={displayTitle(lesson.title)} ondelete={() => deletion.openLesson(group, lesson.key)} />
        </span>
      </li>
    {/each}
  </ol>
</li>

<style>
  .module {
    border-bottom: 1px solid var(--line);
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--s-4);
  }
  .open {
    flex: 1;
    min-width: 0;
    display: grid;
    grid-template-columns: 64px minmax(0, 1fr) minmax(120px, 220px);
    align-items: center;
    gap: var(--s-6);
    padding: var(--s-5) var(--s-2);
    text-align: left;
    border-radius: var(--r-2);
    transition: background-color var(--dur-1) var(--ease-out);
  }
  .open:hover {
    background: var(--surface-1);
  }
  .idx {
    font-family: var(--font-display);
    font-size: var(--fs-xl);
    font-weight: var(--fw-numeral);
    letter-spacing: -0.03em;
    line-height: 1;
    color: var(--text-muted);
  }
  .body {
    display: grid;
    gap: var(--s-1);
    min-width: 0;
  }
  .title {
    font-family: var(--font-display);
    font-weight: var(--fw-display);
    letter-spacing: var(--tracking-title);
    font-size: var(--fs-lg);
    line-height: var(--lh-snug);
  }
  .info {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    font-size: var(--fs-xs);
    color: var(--text-muted);
  }
  .module[data-state="done"] .info {
    color: var(--ok);
  }
  .toggle {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    height: 36px;
    padding: 0 var(--s-3);
    border-radius: var(--r-2);
    color: var(--text-muted);
    font-size: var(--fs-xs);
    white-space: nowrap;
  }
  .toggle:hover {
    color: var(--text);
    background: var(--surface-1);
  }
  .toggle :global(svg) {
    transition: transform var(--dur-2) var(--ease-out);
  }
  .toggle[aria-expanded="true"] :global(svg) {
    transform: rotate(180deg);
  }
  .lessons {
    padding: 0 0 var(--s-5) calc(64px + var(--s-6) + var(--s-2));
  }
  .lessons a {
    display: grid;
    grid-template-columns: 20px minmax(0, 1fr) auto;
    align-items: center;
    gap: var(--s-3);
    padding: var(--s-2) var(--s-3);
    border-radius: var(--r-2);
    color: var(--text);
    text-decoration: none;
    font-size: var(--fs-sm);
  }
  .lessons a:hover {
    background: var(--surface-1);
  }
  .mark {
    color: var(--text-muted);
  }
  .done .mark,
  .done .lstate {
    color: var(--ok);
  }
  .novideo .ltitle {
    color: var(--text-muted);
  }
  .ltitle {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .lstate {
    font-size: var(--fs-xs);
    color: var(--text-muted);
  }
  @media (max-width: 900px) {
    .open {
      grid-template-columns: 44px minmax(0, 1fr);
      gap: var(--s-4);
    }
    .bar {
      grid-column: 2;
    }
    .toggle-label {
      display: none;
    }
    .lessons {
      padding-left: var(--s-2);
    }
  }
  .lrow {
    position: relative;
  }
  .lrow > a {
    padding-right: 56px;
  }
  .menu-slot {
    position: absolute;
    right: var(--s-2);
    top: 50%;
    transform: translateY(-50%);
  }
</style>
