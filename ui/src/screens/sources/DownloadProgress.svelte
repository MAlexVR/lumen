<script lang="ts">
  // Live progress of one running download.
  import ProgressLine from "../../lib/components/ProgressLine.svelte";
  import { t } from "../../lib/i18n/index.svelte";
  import { overallPercent, type DownloadEntry } from "../../lib/logic/downloads";
  import { displayTitle } from "../../lib/logic/title";

  let { entry, groupName }: { entry: DownloadEntry; groupName: string } = $props();
  const pct = $derived(overallPercent(entry));
</script>

<div class="progress">
  <p class="now" aria-live="polite">
    {#if entry.total}
      <span class="course">{displayTitle(entry.course)}</span>
      <span class="sep" aria-hidden="true">/</span>
      <span class="lesson">{displayTitle(entry.lesson)}</span>
    {:else}
      <span class="lesson">{t("dl_preparing")}</span>
    {/if}
  </p>
  <div class="bar">
    <ProgressLine value={pct} thickness={3} label={t("dl_progress_label", { group: displayTitle(groupName) })} />
    <span class="pct num">{pct}%</span>
  </div>
  <p class="count num">{t("dl_progress_lessons", { done: entry.completed, total: entry.total || "–" })}</p>
  {#if entry.paused}
    <p class="paused" role="status">{t("dl_paused_note")}</p>
  {/if}
</div>

<style>
  .progress {
    display: grid;
    gap: var(--s-2);
  }
  .now {
    display: flex;
    gap: var(--s-2);
    min-width: 0;
    font-size: var(--fs-sm);
  }
  .course {
    color: var(--text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 40%;
  }
  .sep {
    color: var(--text-faint);
  }
  .lesson {
    color: var(--text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .bar {
    display: grid;
    grid-template-columns: 1fr auto;
    align-items: center;
    gap: var(--s-4);
  }
  .pct {
    font-size: var(--fs-sm);
    color: var(--lumen);
    min-width: 4ch;
    text-align: right;
  }
  .count {
    font-size: var(--fs-xs);
    color: var(--text-muted);
  }
  .paused {
    color: var(--lumen);
    font-size: var(--fs-sm);
  }
</style>
