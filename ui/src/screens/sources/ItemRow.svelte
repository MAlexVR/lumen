<script lang="ts" generics="Item">
  // One downloadable item (Skool community, Hotmart course): local state,
  // start/cancel, live progress and the result of the last download.
  import Icon from "../../lib/design/Icon.svelte";
  import { t } from "../../lib/i18n/index.svelte";
  import { downloadFor } from "../../lib/logic/downloads";
  import type { LocalStatus } from "../../lib/logic/localStatus";
  import { initials } from "../../lib/logic/poster";
  import { displayTitle } from "../../lib/logic/title";
  import { app } from "../../lib/state/app.svelte";
  import { center, type DownloadItemSession } from "../../lib/state/downloads.svelte";
  import { navigate } from "../../lib/state/router.svelte";
  import type { PlatformId } from "../../lib/types";
  import DownloadProgress from "./DownloadProgress.svelte";
  import ErrorNote from "./ErrorNote.svelte";
  import ReportPanel from "./ReportPanel.svelte";

  interface Props {
    session: DownloadItemSession;
    platform: PlatformId;
    /** Platform key passed to start (Skool slug, Hotmart course id). */
    itemKey: string;
    /** Name as the backend reports it in progress events. */
    name: string;
    coverUrl: string | null;
    /** Short facts under the name (courses, seller, purchase date). */
    facts: string[];
    local: LocalStatus;
    /** Library-relative folder opened from the result panel. */
    folder: string;
    disabledReason?: string | null;
  }
  let { session, platform, itemKey, name, coverUrl, facts, local, folder, disabledReason = null }: Props = $props();
  let coverFailed = $state(false);

  const entry = $derived(downloadFor(center.downloads, { platform, key: itemKey, name }));
  const running = $derived(entry?.status === "running");
  const startError = $derived(session.startErrors[itemKey]);
  const title = $derived(displayTitle(name));
</script>

<li class="item" data-running={running ? "" : undefined}>
  <div class="row">
    <div class="cover" aria-hidden="true">
      {#if coverUrl && !coverFailed}
        <img src={coverUrl} alt="" loading="lazy" referrerpolicy="no-referrer" onerror={() => (coverFailed = true)} />
      {:else}
        <span class="tile">{initials(name)}</span>
      {/if}
    </div>
    <div class="meta">
      <h3>{title}</h3>
      <p>
        {#each facts as fact, i (i)}
          <span class="num">{fact}</span><span class="dot" aria-hidden="true">·</span>
        {/each}
        <span class="local" data-local={local}>{t(`dl_local_${local}`)}</span>
      </p>
    </div>
    <div class="act">
      {#if running && entry}
        <button
          type="button"
          class="btn btn-quiet"
          onclick={() => (entry.paused ? session.resume(entry.id) : session.pause(entry.id))}
          aria-label="{t(entry.paused ? 'dl_resume_download' : 'dl_pause_download')}: {title}"
        >
          <Icon name={entry.paused ? "play" : "pause"} size={16} />{t(entry.paused ? "dl_resume" : "dl_pause")}
        </button>
        <button
          type="button"
          class="btn btn-quiet"
          onclick={() => session.cancel(entry.id)}
          disabled={session.cancelling[entry.id]}
          aria-label="{t('dl_cancel_download')}: {title}"
        >
          <Icon name="close" size={16} />{t(session.cancelling[entry.id] ? "dl_cancelling" : "dl_cancel")}
        </button>
      {:else}
        <button
          type="button"
          class="btn btn-quiet"
          onclick={() => session.start(itemKey)}
          disabled={session.starting[itemKey] || disabledReason !== null}
          aria-label="{t(local === 'none' ? 'dl_download' : 'dl_update')}: {title}"
        >
          <Icon name="download" size={16} />
          {t(session.starting[itemKey] ? "dl_starting" : local === "none" ? "dl_download" : "dl_update")}
        </button>
      {/if}
    </div>
  </div>

  {#if running && entry}
    <div class="detail"><DownloadProgress {entry} groupName={name} /></div>
  {:else if entry?.report}
    <div class="detail">
      <ReportPanel
        {entry}
        report={entry.report}
        onopen={() => void app.api?.openFolder(folder).catch(() => {})}
        onretry={() => session.start(itemKey)}
        ondismiss={() => session.dismiss(entry.id)}
      />
    </div>
  {/if}

  {#if disabledReason}
    <p class="detail muted" role="status">{disabledReason}</p>
  {/if}

  {#if startError}
    <div class="detail">
      <ErrorNote error={startError} platform={platform}>
        {#if startError.code === "no_library"}
          <button type="button" class="btn btn-quiet" onclick={() => navigate({ name: "settings" })}>{t("dl_go_settings")}</button>
        {/if}
      </ErrorNote>
    </div>
  {/if}
</li>

<style>
  .item {
    border-bottom: 1px solid var(--line);
    padding: var(--s-5) 0;
    display: grid;
    gap: var(--s-4);
  }
  .row {
    display: grid;
    grid-template-columns: 56px minmax(0, 1fr) auto;
    gap: var(--s-5);
    align-items: center;
  }
  .cover {
    width: 56px;
    height: 56px;
    border-radius: var(--r-2);
    overflow: hidden;
    background: var(--surface-2);
    outline: 1px solid var(--line-soft);
    outline-offset: -1px;
  }
  img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .tile {
    display: grid;
    place-items: center;
    width: 100%;
    height: 100%;
    font-family: var(--font-display);
    font-size: 1.125rem;
    font-weight: var(--fw-display);
    color: var(--ink-strong);
  }
  [data-running] .cover {
    box-shadow:
      0 0 0 1px var(--lumen),
      0 0 24px 2px var(--lumen-glow);
  }
  h3 {
    font-size: var(--fs-lg);
    line-height: var(--lh-snug);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .meta {
    min-width: 0;
  }
  .meta p {
    display: flex;
    flex-wrap: wrap;
    gap: 0 var(--s-2);
    font-size: var(--fs-xs);
    color: var(--text-muted);
  }
  .dot {
    color: var(--text-faint);
  }
  .local[data-local="complete"] {
    color: var(--ok);
  }
  .local[data-local="partial"] {
    color: var(--moon);
  }
  .muted {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .detail {
    padding-left: calc(56px + var(--s-5));
  }
  @media (max-width: 760px) {
    .detail {
      padding-left: 0;
    }
  }
</style>
