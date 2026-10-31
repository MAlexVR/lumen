<script lang="ts">
  // YouTube flow inside Descargas: paste a link, preview it, choose videos and
  // quality, download. Progress and results reuse the shared components.
  import Icon from "../../lib/design/Icon.svelte";
  import { t } from "../../lib/i18n/index.svelte";
  import { initials } from "../../lib/logic/poster";
  import { formatTime } from "../../lib/logic/time";
  import { displayTitle } from "../../lib/logic/title";
  import { QUALITIES, selectionTotals, youtubeHint } from "../../lib/logic/youtube";
  import { app } from "../../lib/state/app.svelte";
  import { center, youtube } from "../../lib/state/downloads.svelte";
  import { navigate } from "../../lib/state/router.svelte";
  import DownloadProgress from "./DownloadProgress.svelte";
  import ErrorNote from "./ErrorNote.svelte";
  import ReportPanel from "./ReportPanel.svelte";

  let url = $state("");
  let thumbFailed = $state(false);
  let timer: ReturnType<typeof setTimeout> | undefined;
  const uid = $props.id();

  const hint = $derived(youtubeHint(url));
  const p = $derived(youtube.preview);
  const items = $derived(p?.items ?? []);
  const totals = $derived(selectionTotals(items, youtube.selection));
  const entries = $derived(
    Object.values(center.downloads)
      .filter((d) => d.platform === "youtube")
      .sort((a, b) => Number(b.id.replace(/\D+/g, "")) - Number(a.id.replace(/\D+/g, ""))),
  );
  const canDownload = $derived(!!p && !youtube.starting && (p.kind === "video" || youtube.selection.length > 0));

  /** Resolve shortly after a valid link appears (typing or pasting); Enter resolves now. */
  function schedule(delay: number) {
    clearTimeout(timer);
    if (youtubeHint(url).state !== "valid") return;
    timer = setTimeout(() => youtube.resolve(url), delay);
  }

  function onInput() {
    thumbFailed = false;
    if (!url.trim()) youtube.clear();
    schedule(700);
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      clearTimeout(timer);
      void youtube.resolve(url);
    }
  }

  async function paste() {
    try {
      const text = await navigator.clipboard.readText();
      if (text) {
        url = text.trim();
        thumbFailed = false;
        schedule(150);
      }
    } catch {
      /* clipboard not readable: the field still accepts Ctrl+V */
    }
  }

  /** Title known from this session or the progress events; otherwise a calm label, never the raw link. */
  const titleOf = (key: string | null, course: string) => {
    const known = (key && youtube.titles[key]) || course;
    if (known) return displayTitle(known);
    return t(key?.includes("list=") ? "yt_kind_playlist" : "yt_kind_video");
  };
</script>

<div class="flow">
  <div class="url" role="search">
    <label for="yt-url-{uid}" class="eyebrow">{t("yt_url_label")}</label>
    <div class="field">
      <Icon name="link" size={18} />
      <input
        id="yt-url-{uid}"
        type="url"
        inputmode="url"
        bind:value={url}
        oninput={onInput}
        onpaste={() => schedule(150)}
        onkeydown={onKey}
        placeholder={t("yt_url_placeholder")}
        autocomplete="off"
        spellcheck="false"
        aria-describedby="yt-hint-{uid}"
      />
      <button type="button" class="btn btn-quiet" onclick={paste}>{t("yt_paste")}</button>
      <button
        type="button"
        class="btn btn-quiet"
        onclick={() => youtube.resolve(url)}
        disabled={!url.trim() || youtube.resolving}
      >
        {t("yt_resolve")}
      </button>
    </div>
    <p id="yt-hint-{uid}" class="hint" aria-live="polite">
      {#if youtube.resolving}
        <span class="pulse" aria-hidden="true"></span>{t("yt_resolving")}
        <button type="button" class="link" onclick={() => youtube.cancelResolve()}>{t("yt_cancel_resolve")}</button>
      {:else if hint.state === "not_url"}
        {t("yt_hint_not_url")}
      {:else if hint.state === "other_site"}
        {t("yt_hint_other_site")}
      {:else if hint.state === "unsupported"}
        {t("yt_hint_unsupported")}
      {/if}
    </p>
  </div>

  {#if youtube.error}
    <ErrorNote error={youtube.error} />
  {/if}

  {#if p}
    <section class="preview" aria-labelledby="yt-title-{uid}">
      <div class="thumb" aria-hidden="true">
        {#if p.thumbnailUrl && !thumbFailed}
          <img src={p.thumbnailUrl} alt="" referrerpolicy="no-referrer" onerror={() => (thumbFailed = true)} />
        {:else}
          <span class="tile">{initials(p.title)}</span>
        {/if}
        <span class="kind">{t(p.kind === "video" ? "yt_kind_video" : "yt_kind_playlist")}</span>
      </div>
      <div class="meta">
        <h2 id="yt-title-{uid}">{displayTitle(p.title)}</h2>
        <p class="facts num">
          <span>{t("yt_by", { channel: p.channel })}</span>
          {#if p.kind === "video" && p.durationSecs}<span>· {formatTime(p.durationSecs)}</span>{/if}
          {#if p.kind === "playlist"}<span>· {t("yt_videos_n", { n: items.length })}</span>{/if}
        </p>
        <p class="folder"><span class="eyebrow">{t("yt_saved_to")}</span><code>YouTube/{p.folder}</code></p>
      </div>
    </section>

    {#if p.kind === "playlist"}
      <section class="items" aria-labelledby="yt-items-{uid}">
        <div class="items-head">
          <h3 id="yt-items-{uid}" class="eyebrow">{t("yt_items_label")}</h3>
          <span class="count num" aria-live="polite">
            {t("yt_selected", { count: totals.count, available: totals.available })} ·
            {t("yt_total", { time: formatTime(totals.seconds) })}
            {#if totals.unknown}{t("yt_total_unknown", { n: totals.unknown })}{/if}
          </span>
          <span class="spacer"></span>
          <button type="button" class="link" onclick={() => youtube.select({ type: "all" })}>{t("yt_select_all")}</button>
          <button type="button" class="link" onclick={() => youtube.select({ type: "none" })}>{t("yt_select_none")}</button>
        </div>
        <ol>
          {#each items as item, i (item.id)}
            <li class:off={!item.available}>
              <label>
                <input
                  type="checkbox"
                  checked={youtube.selection.includes(item.id)}
                  disabled={!item.available}
                  onchange={() => youtube.select({ type: "toggle", id: item.id })}
                />
                <span class="n num">{String(i + 1).padStart(2, "0")}</span>
                <span class="ititle">
                  {displayTitle(item.title)}
                  {#if !item.available}<span class="reason">{t("yt_unavailable", { reason: item.reason ?? "" })}</span>{/if}
                </span>
                <span class="dur num">{item.durationSecs ? formatTime(item.durationSecs) : ""}</span>
              </label>
            </li>
          {/each}
        </ol>
      </section>
    {/if}

    <section class="quality" aria-labelledby="yt-q-{uid}">
      <h3 id="yt-q-{uid}" class="eyebrow">{t("yt_quality")}</h3>
      <div class="segmented" role="radiogroup" aria-labelledby="yt-q-{uid}">
        {#each QUALITIES as q (q)}
          <button type="button" role="radio" aria-checked={youtube.quality === q} onclick={() => youtube.setQuality(q)}>
            {t(`yt_q_${q}`)}
          </button>
        {/each}
      </div>
      <p class="help">{t(`yt_q_help_${youtube.quality}`)}</p>
    </section>

    <div class="go">
      <button type="button" class="btn btn-primary" disabled={!canDownload} onclick={() => youtube.start()}>
        <Icon name="download" size={16} />
        {youtube.starting
          ? t("yt_starting")
          : p.kind === "playlist"
            ? t("yt_download_n", { n: youtube.selection.length })
            : t("yt_download")}
      </button>
      {#if p.kind === "playlist" && youtube.selection.length === 0}
        <span class="help">{t("yt_nothing_selected")}</span>
      {/if}
    </div>
    {#if youtube.startError}
      <ErrorNote error={youtube.startError}>
        {#if youtube.startError.code === "no_library"}
          <button type="button" class="btn btn-quiet" onclick={() => navigate({ name: "settings" })}>{t("dl_go_settings")}</button>
        {/if}
      </ErrorNote>
    {/if}
  {:else if !youtube.resolving && !youtube.error}
    <div class="empty">
      <p class="strong">{t("yt_empty_title")}</p>
      <p class="muted">{t("yt_empty_body")}</p>
    </div>
  {/if}

  {#if entries.length}
    <section class="downloads" aria-labelledby="yt-dl-{uid}">
      <h3 id="yt-dl-{uid}" class="eyebrow">{t("yt_downloads")}</h3>
      <ul>
        {#each entries as entry (entry.id)}
          <li>
            <div class="row">
              <span class="dtitle">{titleOf(entry.key, entry.course)}</span>
              {#if entry.status === "running"}
                <button
                  type="button"
                  class="btn btn-quiet"
                  onclick={() => (entry.paused ? youtube.resume(entry.id) : youtube.pause(entry.id))}
                >
                  <Icon name={entry.paused ? "play" : "pause"} size={16} />{t(entry.paused ? "dl_resume" : "dl_pause")}
                </button>
                <button
                  type="button"
                  class="btn btn-quiet"
                  onclick={() => youtube.cancel(entry.id)}
                  disabled={youtube.cancelling[entry.id]}
                >
                  <Icon name="close" size={16} />{t(youtube.cancelling[entry.id] ? "dl_cancelling" : "dl_cancel")}
                </button>
              {/if}
            </div>
            {#if entry.status === "running"}
              <DownloadProgress {entry} groupName={titleOf(entry.key, entry.course)} />
            {:else if entry.report}
              <ReportPanel
                {entry}
                report={entry.report}
                onopen={() => void app.api?.openFolder(`YouTube/${(entry.key && youtube.folders[entry.key]) || entry.course}`).catch(() => {})}
                onretry={() => entry.key && youtube.retry(entry.key)}
                ondismiss={() => youtube.dismiss(entry.id)}
              />
            {/if}
          </li>
        {/each}
      </ul>
    </section>
  {/if}
</div>

<style>
  .flow {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: var(--s-8);
    max-width: 880px;
  }
  .url {
    display: grid;
    gap: var(--s-2);
  }
  .url label {
    font-family: var(--font-ui);
  }
  .field {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    padding: 0 0 0 var(--s-3);
    border-bottom: 1px solid var(--line);
    color: var(--text-muted);
    transition: border-color var(--dur-2) var(--ease-out);
  }
  .field:focus-within {
    border-color: var(--lumen);
    color: var(--text);
  }
  .field input {
    flex: 1;
    min-width: 0;
    height: 48px;
    background: none;
    border: 0;
    outline: none;
    font-size: var(--fs-md);
    color: var(--text);
  }
  .field input::placeholder {
    color: var(--text-muted);
  }
  .field .btn {
    height: 34px;
  }
  .hint {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-height: 1.5em;
    font-size: var(--fs-sm);
    color: var(--text-muted);
  }
  .pulse {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--lumen);
    animation: pulse 1.6s var(--ease-out) infinite;
  }
  @keyframes pulse {
    0% {
      box-shadow: 0 0 0 0 var(--lumen-glow);
    }
    100% {
      box-shadow: 0 0 0 10px transparent;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .pulse {
      animation: none;
    }
  }
  .link {
    color: var(--text-muted);
    font-size: var(--fs-sm);
    text-decoration: underline;
    text-underline-offset: 3px;
  }
  .link:hover {
    color: var(--text);
  }
  .preview {
    display: grid;
    grid-template-columns: minmax(180px, 280px) minmax(0, 1fr);
    gap: var(--s-6);
    align-items: center;
  }
  .thumb {
    position: relative;
    aspect-ratio: 16 / 9;
    border-radius: var(--r-2);
    overflow: hidden;
    background: var(--surface-2);
    outline: 1px solid var(--line);
    outline-offset: -1px;
  }
  .thumb img {
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
    font-weight: var(--fw-display);
    font-size: 2rem;
    color: var(--ink-strong);
  }
  .kind {
    position: absolute;
    left: var(--s-2);
    bottom: var(--s-2);
    font-size: var(--fs-2xs);
    letter-spacing: var(--tracking-label);
    text-transform: uppercase;
    background: var(--scrim);
    color: var(--text);
    padding: 2px var(--s-2);
    border-radius: var(--r-1);
  }
  .meta {
    display: grid;
    gap: var(--s-2);
    min-width: 0;
  }
  .meta h2 {
    font-size: var(--fs-xl);
  }
  .facts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-1);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .folder {
    display: grid;
    gap: 2px;
    margin-top: var(--s-2);
  }
  .folder .eyebrow {
    font-family: var(--font-ui);
  }
  code {
    font-family: ui-monospace, "SFMono-Regular", "Cascadia Mono", "DejaVu Sans Mono", monospace;
    font-size: var(--fs-sm);
    color: var(--text);
    overflow-wrap: anywhere;
  }
  .items-head {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--s-2) var(--s-4);
    padding-bottom: var(--s-2);
    border-bottom: 1px solid var(--line);
  }
  .items-head h3 {
    font-family: var(--font-ui);
  }
  .count {
    font-size: var(--fs-xs);
    color: var(--text-muted);
  }
  .spacer {
    flex: 1;
  }
  .items ol {
    display: grid;
  }
  .items li {
    border-bottom: 1px solid var(--line-soft);
  }
  .items label {
    display: grid;
    grid-template-columns: 20px 28px minmax(0, 1fr) auto;
    align-items: start;
    gap: var(--s-3);
    padding: var(--s-3) var(--s-2);
    font-size: var(--fs-sm);
    cursor: pointer;
  }
  .items label:hover {
    background: var(--surface-1);
  }
  .items input {
    margin-top: 3px;
    accent-color: var(--lumen);
  }
  .n {
    color: var(--text-muted);
  }
  .ititle {
    display: grid;
    gap: 2px;
    color: var(--text);
  }
  .off .ititle,
  .off .n {
    color: var(--text-faint);
  }
  .off label {
    cursor: default;
  }
  .reason {
    font-size: var(--fs-xs);
    color: var(--text-muted);
  }
  .dur {
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .quality {
    display: grid;
    gap: var(--s-3);
  }
  .quality h3 {
    font-family: var(--font-ui);
  }
  .segmented {
    display: inline-flex;
    flex-wrap: wrap;
    justify-self: start;
    border: 1px solid var(--line);
    border-radius: var(--r-2);
    padding: 2px;
  }
  .segmented button {
    height: 34px;
    padding: 0 var(--s-4);
    border-radius: var(--r-1);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .segmented button[aria-checked="true"] {
    background: var(--surface-3);
    color: var(--text);
  }
  .help {
    font-size: var(--fs-sm);
    color: var(--text-muted);
  }
  .go {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-4);
  }
  .empty {
    display: grid;
    gap: var(--s-2);
    padding: var(--s-6) 0;
    border-top: 1px solid var(--line);
  }
  .strong {
    color: var(--text);
    font-size: var(--fs-md);
  }
  .muted {
    color: var(--text-muted);
    max-width: 56ch;
  }
  .downloads {
    display: grid;
    gap: var(--s-3);
  }
  .downloads h3 {
    font-family: var(--font-ui);
  }
  .downloads ul {
    border-top: 1px solid var(--line);
  }
  .downloads li {
    display: grid;
    gap: var(--s-3);
    padding: var(--s-4) 0;
    border-bottom: 1px solid var(--line);
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--s-3);
  }
  .dtitle {
    flex: 1;
    min-width: 0;
    font-family: var(--font-display);
    font-weight: var(--fw-display);
    font-size: var(--fs-lg);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  @media (max-width: 900px) {
    .preview {
      grid-template-columns: 140px minmax(0, 1fr);
      gap: var(--s-4);
      align-items: start;
    }
    .meta h2 {
      font-size: var(--fs-lg);
    }
    .tile {
      font-size: 1.25rem;
    }
    .kind {
      display: none;
    }
  }
</style>
