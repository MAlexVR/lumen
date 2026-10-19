<script lang="ts">
  import { displayTitle } from "../../lib/logic/title";
  // Lessons of the current course, grouped by module; the current one glows.
  import { tick } from "svelte";
  import Icon from "../../lib/design/Icon.svelte";
  import { t } from "../../lib/i18n/index.svelte";
  import { courseStats } from "../../lib/logic/progress";
  import { href } from "../../lib/state/router.svelte";
  import type { Course } from "../../lib/types";

  interface Props {
    group: string;
    course: Course;
    currentKey: string;
    watched: ReadonlySet<string>;
    onpick?: () => void;
  }
  let { group, course, currentKey, watched, onpick }: Props = $props();
  let list: HTMLElement | undefined = $state();
  const stats = $derived(courseStats(course, watched));

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
    <span class="eyebrow">{t("lessons")}</span>
    <a class="course" href={href({ name: "course", group, course: course.index })}>{displayTitle(course.title)}</a>
    <span class="count num">{t("lessons_count", { done: stats.done, total: stats.total })} · {stats.pct}%</span>
  </header>
  {#each course.sections as section (section.index)}
    <section aria-label={displayTitle(section.title)}>
      {#if course.sections.length > 1}
        <h3><span class="num idx">{String(section.index).padStart(2, "0")}</span>{displayTitle(section.title)}</h3>
      {/if}
      <ol>
        {#each section.lessons as lesson (lesson.key)}
          {@const done = watched.has(lesson.key)}
          {@const current = lesson.key === currentKey}
          <li>
            <a
              href={href({ name: "watch", group, key: lesson.key })}
              aria-current={current ? "true" : undefined}
              class:done
              class:novideo={!lesson.hasVideo}
              onclick={onpick}
            >
              <span class="mark" aria-hidden="true">
                <Icon name={done ? "check" : lesson.hasVideo ? "play" : "noVideo"} size={14} />
              </span>
              <span class="title">{displayTitle(lesson.title)}</span>
              <span class="visually-hidden">
                {current ? t("current") : ""}
                {done ? t("watched") : lesson.hasVideo ? t("not_watched") : t("no_video")}
              </span>
            </a>
          </li>
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
</style>
