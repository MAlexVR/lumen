<script lang="ts">
  // Ajustes › Actualizaciones: check the releases, open the release page and,
  // while the repository is private, keep a read-only access token. Nothing is
  // downloaded or installed by the app.
  import { onDestroy, onMount, tick } from "svelte";
  import Icon from "../../lib/design/Icon.svelte";
  import { i18n, t } from "../../lib/i18n/index.svelte";
  import { formatBytes } from "../../lib/logic/deletion";
  import {
    initialUpdate,
    notesBlocks,
    readLastChecked,
    reduceUpdate,
    relativeTime,
    tokenDraft,
    updateErrorKey,
    writeLastChecked,
    type UpdateAction,
    type UpdateState,
  } from "../../lib/logic/updates";
  import { app } from "../../lib/state/app.svelte";

  const uid = $props.id();
  let upd = $state<UpdateState>(initialUpdate);
  let lastChecked = $state<string | null>(readLastChecked(localStorage));
  let now = $state(Date.now());
  let tokenSaved = $state(false);
  let tokenOpen = $state(false);
  let token = $state("");
  let tokenBusy = $state(false);
  let tokenFailed = $state(false);
  let confirmingClear = $state(false);
  let includePre = $state(true);
  let copied = $state(false);
  let openFailed = $state(false);
  let heading: HTMLElement | undefined = $state();
  let tick60: ReturnType<typeof setInterval> | undefined;

  const dispatch = (a: UpdateAction) => (upd = reduceUpdate(upd, a));
  const version = $derived(`${/^\d/.test(app.version) ? "v" : ""}${app.version}`);
  const checking = $derived(upd.step === "checking");
  const draft = $derived(tokenDraft(token));

  onMount(async () => {
    tick60 = setInterval(() => (now = Date.now()), 30_000);
    const api = app.api;
    if (!api) return;
    tokenSaved = (await api.updateTokenStatus().catch(() => ({ saved: false }))).saved;
    includePre = (await api.updateSettings().catch(() => ({ includePrereleases: true }))).includePrereleases;
  });
  onDestroy(() => {
    clearInterval(tick60);
    token = "";
  });

  async function check() {
    const api = app.api;
    if (!api || checking) return;
    openFailed = false;
    dispatch({ type: "check" });
    try {
      const info = await api.updateCheck();
      lastChecked = info.checkedAt;
      writeLastChecked(localStorage, info.checkedAt);
      now = Date.now();
      dispatch({ type: "done", info });
    } catch (e) {
      dispatch({ type: "failed", error: e, tokenSaved });
      if (upd.step === "error" && (upd.code === "update_unauthorized" || upd.code === "invalid_token") && !tokenSaved) tokenOpen = true;
    }
    await tick();
    heading?.focus();
  }

  async function saveToken() {
    const api = app.api;
    if (!api || !draft.valid || tokenBusy) return;
    tokenBusy = true;
    tokenFailed = false;
    try {
      await api.updateTokenSave(draft.value);
      tokenSaved = true;
      token = "";
      tokenOpen = false;
    } catch {
      tokenFailed = true;
    } finally {
      tokenBusy = false;
    }
  }

  async function clearToken() {
    confirmingClear = false;
    try {
      await app.api?.updateTokenClear();
      tokenSaved = false;
    } catch {
      /* the saved state stays as it was */
    }
  }

  async function togglePre() {
    const value = !includePre;
    includePre = value;
    try {
      await app.api?.updateSetIncludePrereleases(value);
    } catch {
      includePre = !value;
    }
  }

  async function openRelease(url: string | null) {
    if (!url?.startsWith("https://")) return;
    openFailed = false;
    try {
      await app.api?.openUrl(url);
    } catch {
      openFailed = true;
    }
  }

  async function copyLink(url: string) {
    try {
      await navigator.clipboard.writeText(url);
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {
      /* nothing to copy into */
    }
  }

  const date = (iso: string) => new Date(iso).toLocaleDateString(i18n.locale, { year: "numeric", month: "long", day: "numeric" });
  const time = (iso: string) => new Date(iso).toLocaleTimeString(i18n.locale, { hour: "2-digit", minute: "2-digit" });
</script>

<section class="row" aria-labelledby="upd-h-{uid}">
  <h2 id="upd-h-{uid}" class="eyebrow">{t("upd_title")}</h2>
  <div class="body">
    <div class="head">
      <div class="ver">
        <span class="label">{t("upd_current")}</span>
        <span class="num">{version}</span>
        <span class="muted">{lastChecked ? t("upd_last", { when: relativeTime(lastChecked, now, i18n.locale) }) : t("upd_never")}</span>
      </div>
      <button type="button" class="btn btn-quiet" onclick={check} disabled={checking}>{t(checking ? "upd_checking" : "upd_check")}</button>
    </div>

    <div class="result" aria-live="polite">
      {#if upd.step === "checking"}
        <p class="muted">{t("upd_checking")}</p>
      {:else if upd.step === "up_to_date"}
        <p class="ok"><Icon name="check" size={16} /><span tabindex="-1" bind:this={heading}>{t("upd_up_to_date", { version: upd.info.current })}</span></p>
      {:else if upd.step === "available"}
        {@const info = upd.info}
        <div class="card" role="group" aria-labelledby="upd-new-{uid}">
          <h3 id="upd-new-{uid}" tabindex="-1" bind:this={heading}>
            {t("upd_available", { version: info.latest ?? "" })}
            {#if info.isPrerelease}<span class="tag">{t("upd_prerelease")}</span>{/if}
          </h3>
          {#if info.publishedAt}<p class="muted">{t("upd_published", { date: date(info.publishedAt) })}</p>{/if}
          {#if info.asset}
            <p class="file">
              <span class="label">{t("upd_file")}</span>
              <code>{info.asset.name}</code> <span class="num muted">{formatBytes(info.asset.size, false, i18n.locale)}</span>
            </p>
          {/if}
          <details class="notes">
            <summary>{t("upd_notes")}</summary>
            {#each notesBlocks(info.notes) as b, i (i)}
              {#if b.kind === "heading"}<h4>{b.text}</h4>
              {:else if b.kind === "item"}<p class="item">{b.text}</p>
              {:else}<p>{b.text}</p>{/if}
            {:else}
              <p class="muted">{t("upd_no_notes")}</p>
            {/each}
          </details>
          <div class="actions">
            {#if info.htmlUrl}
              <button type="button" class="btn btn-primary" onclick={() => openRelease(info.htmlUrl)}>
                <Icon name="external" size={16} />{t("upd_open")}
              </button>
            {/if}
            {#if info.asset}
              {@const url = info.asset.downloadUrl}
              <button type="button" class="btn btn-quiet" onclick={() => copyLink(url)}>
                <Icon name={copied ? "check" : "link"} size={16} />{t(copied ? "upd_copied" : "upd_copy")}
              </button>
            {/if}
          </div>
          {#if openFailed}<p class="muted" role="status">{t("upd_open_failed")}</p>{/if}
        </div>
      {:else if upd.step === "error"}
        <div class="error">
          <Icon name="alert" size={18} />
          <div>
            <p class="what" tabindex="-1" bind:this={heading}>{t("upd_err_title")}</p>
            <p class="next">
              {#if upd.code === "rate_limited" && upd.resetAt}
                {t("upd_err_rate_limited_at", { time: time(upd.resetAt) })}
              {:else}
                {t(updateErrorKey(upd.code, upd.tokenSaved))}
              {/if}
            </p>
          </div>
        </div>
      {/if}
    </div>

    <div class="opt">
      <label class="switch">
        <input type="checkbox" role="switch" checked={includePre} onchange={togglePre} />
        <span>{t("upd_prereleases")}</span>
      </label>
      <p class="muted">{t("upd_prereleases_hint")}</p>
    </div>

    <details class="access" bind:open={tokenOpen}>
      <summary>
        {t("upd_access")}
        <span class="muted state">· <Icon name={tokenSaved ? "check" : "alert"} size={14} />{t(tokenSaved ? "upd_token_saved" : "upd_token_none")}</span>
      </summary>
      <p class="muted">{t("upd_access_hint")}</p>
      <ol class="steps">
        <li>{t("upd_step1")}</li>
        <li>{t("upd_step2")}</li>
        <li>{t("upd_step3")}</li>
      </ol>
      <form
        class="token"
        onsubmit={(e) => {
          e.preventDefault();
          void saveToken();
        }}
      >
        <label for="upd-token-{uid}">{t("upd_token_label")}</label>
        <div class="inline">
          <input
            id="upd-token-{uid}"
            type="password"
            autocomplete="off"
            spellcheck="false"
            bind:value={token}
            aria-invalid={tokenFailed}
          />
          <button type="submit" class="btn btn-quiet" disabled={!draft.valid || tokenBusy}>
            {t(tokenBusy ? "upd_token_saving" : "upd_token_save")}
          </button>
        </div>
        {#if tokenFailed}<p class="what-err" role="alert">{t("upd_token_failed")}</p>{/if}
      </form>
      {#if tokenSaved}
        {#if confirmingClear}
          <div class="confirm" role="group">
            <p class="muted">{t("upd_token_clear_confirm")}</p>
            <div class="actions">
              <button type="button" class="btn btn-quiet" onclick={clearToken}>{t("upd_token_clear_yes")}</button>
              <button type="button" class="btn link" onclick={() => (confirmingClear = false)}>{t("dl_cancel")}</button>
            </div>
          </div>
        {:else}
          <div><button type="button" class="btn link" onclick={() => (confirmingClear = true)}>{t("upd_token_clear")}</button></div>
        {/if}
      {/if}
    </details>
  </div>
</section>

<style>
  .row {
    display: grid;
    grid-template-columns: minmax(140px, 220px) 1fr;
    gap: var(--s-6);
    padding: var(--s-6) 0;
    border-top: 1px solid var(--line);
    align-items: start;
  }
  .row h2 {
    font-family: var(--font-ui);
    padding-top: var(--s-2);
  }
  .body {
    display: grid;
    gap: var(--s-5);
    min-width: 0;
  }
  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-3);
  }
  .ver {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--s-2) var(--s-3);
    font-size: var(--fs-sm);
  }
  .label,
  .muted {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .ok {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    color: var(--ok);
    font-size: var(--fs-sm);
  }
  .ok span,
  .what,
  h3 {
    outline-offset: 4px;
  }
  .card {
    display: grid;
    gap: var(--s-3);
    padding: var(--s-5);
    background: var(--surface-1);
    border: 1px solid var(--line);
    border-left: 2px solid var(--lumen);
    border-radius: var(--r-3);
  }
  h3 {
    font-size: var(--fs-lg);
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-3);
  }
  .tag {
    font-family: var(--font-ui);
    font-size: var(--fs-xs);
    font-weight: 500;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--lumen);
    border: 1px solid var(--lumen);
    border-radius: var(--r-2);
    padding: 1px var(--s-2);
  }
  .file {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--s-2);
    font-size: var(--fs-sm);
  }
  code {
    font-family: ui-monospace, "SFMono-Regular", "Cascadia Mono", "DejaVu Sans Mono", monospace;
    font-size: var(--fs-sm);
    overflow-wrap: anywhere;
  }
  summary {
    cursor: pointer;
    font-size: var(--fs-sm);
    color: var(--text);
    border-radius: var(--r-2);
  }
  summary:focus-visible {
    outline: 2px solid var(--lumen);
    outline-offset: 3px;
  }
  .notes {
    display: grid;
    gap: var(--s-2);
  }
  .notes h4 {
    font-family: var(--font-ui);
    font-size: var(--fs-sm);
    font-weight: 600;
    margin-top: var(--s-2);
  }
  .notes p {
    font-size: var(--fs-sm);
    color: var(--text-muted);
    overflow-wrap: anywhere;
  }
  .notes .item {
    padding-left: var(--s-4);
    position: relative;
  }
  .notes .item::before {
    content: "–";
    position: absolute;
    left: 0;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-2);
  }
  .error {
    display: flex;
    align-items: flex-start;
    gap: var(--s-3);
    padding: var(--s-3) var(--s-4);
    border-left: 2px solid var(--danger);
    background: var(--surface-1);
    border-radius: 0 var(--r-2) var(--r-2) 0;
  }
  .error > :global(svg) {
    color: var(--danger);
    margin-top: 2px;
    flex: none;
  }
  .error > div {
    display: grid;
    gap: 2px;
    min-width: 0;
  }
  .what {
    font-size: var(--fs-sm);
    font-weight: 500;
  }
  .next {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .opt {
    display: grid;
    gap: var(--s-1);
    padding-top: var(--s-4);
    border-top: 1px solid var(--line-soft);
  }
  .switch {
    display: inline-flex;
    align-items: center;
    gap: var(--s-3);
    font-size: var(--fs-sm);
    cursor: pointer;
  }
  .switch input {
    accent-color: var(--lumen);
    width: 16px;
    height: 16px;
  }
  .switch input:focus-visible {
    outline: 2px solid var(--lumen);
    outline-offset: 3px;
  }
  .access {
    display: grid;
    gap: var(--s-3);
    padding-top: var(--s-4);
    border-top: 1px solid var(--line-soft);
  }
  .access[open] > summary {
    margin-bottom: var(--s-1);
  }
  .state {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
  }
  .steps {
    margin: 0;
    padding-left: var(--s-5);
    list-style: decimal;
    display: grid;
    gap: var(--s-1);
    font-size: var(--fs-sm);
    color: var(--text-muted);
  }
  .token {
    display: grid;
    gap: var(--s-2);
    font-size: var(--fs-sm);
  }
  .inline {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-2);
  }
  .inline input {
    flex: 1;
    min-width: 200px;
    background: var(--surface-1);
    color: var(--text);
    border: 1px solid var(--line);
    border-radius: var(--r-2);
    padding: var(--s-2) var(--s-3);
    font: inherit;
  }
  .inline input:focus-visible {
    outline: 2px solid var(--lumen);
    outline-offset: 2px;
  }
  .what-err {
    color: var(--danger);
  }
  .confirm {
    display: grid;
    gap: var(--s-2);
  }
  .link {
    color: var(--text-muted);
    padding: 0 var(--s-3);
  }
  .link:hover {
    color: var(--text);
  }
  @media (max-width: 760px) {
    .row {
      grid-template-columns: 1fr;
      gap: var(--s-3);
    }
  }
</style>
