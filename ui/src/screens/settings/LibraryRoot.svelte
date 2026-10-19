<script lang="ts">
  // Library folder: where it is, where it came from, whether it is usable,
  // and a pick → review → confirm flow. Existing downloads are never moved.
  import Icon from "../../lib/design/Icon.svelte";
  import { t } from "../../lib/i18n/index.svelte";
  import {
    initialPicker,
    middleTruncate,
    reducePicker,
    rootErrorKind,
    summaryMessage,
    type PickerAction,
    type PickerState,
  } from "../../lib/logic/libraryRoot";
  import { app } from "../../lib/state/app.svelte";

  let picker = $state<PickerState>(initialPicker);
  let confirmingReset = $state(false);
  let copied = $state(false);
  let openFailed = $state(false);
  let review: HTMLElement | undefined = $state();
  let changeButton: HTMLButtonElement | undefined = $state();
  const uid = $props.id();

  const info = $derived(app.rootInfo);
  const dispatch = (a: PickerAction) => (picker = reducePicker(picker, a));

  async function change() {
    const api = app.api;
    if (!api) return;
    dispatch({ type: "pick" });
    let path: string | null;
    try {
      path = await api.libraryPickRoot();
    } catch (e) {
      dispatch({ type: "failed", kind: rootErrorKind(e) });
      return;
    }
    dispatch({ type: "picked", path });
    if (!path) return;
    queueMicrotask(() => review?.focus());
    try {
      dispatch({ type: "summary", summary: await api.librarySummary(path) });
    } catch {
      dispatch({ type: "summary", summary: { platforms: [], hasLibrary: false } });
    }
  }

  async function use() {
    const api = app.api;
    if (!api || picker.step !== "reviewing") return;
    const path = picker.path;
    dispatch({ type: "confirm" });
    try {
      app.rootInfo = await api.librarySetRoot(path);
      dispatch({ type: "saved" });
      changeButton?.focus();
    } catch (e) {
      dispatch({ type: "failed", kind: rootErrorKind(e) });
    }
  }

  async function reset() {
    confirmingReset = false;
    const api = app.api;
    if (!api) return;
    try {
      app.rootInfo = await api.libraryResetRoot();
    } catch (e) {
      dispatch({ type: "failed", kind: rootErrorKind(e) });
    }
  }

  async function copy() {
    if (!info?.path) return;
    try {
      await navigator.clipboard.writeText(info.path);
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {
      /* the path stays selectable */
    }
  }

  async function openFolder() {
    openFailed = false;
    try {
      await app.api?.openFolder();
    } catch {
      openFailed = true;
    }
  }

  function cancel() {
    dispatch({ type: "cancel" });
    changeButton?.focus();
  }
</script>

<section class="row" aria-labelledby="lr-h-{uid}">
  <h2 id="lr-h-{uid}" class="eyebrow">{t("lr_title")}</h2>
  <div class="body">
    {#if info?.path}
      <div class="path-row">
        <code class="path" title={info.path}>{middleTruncate(info.path, 56)}</code>
        <button type="button" class="icon-btn" onclick={copy} aria-label={t(copied ? "lr_copied" : "lr_copy")} title={t(copied ? "lr_copied" : "lr_copy")}>
          <Icon name={copied ? "check" : "file"} size={16} />
        </button>
      </div>
      <ul class="facts">
        <li><span class="label">{t("lr_source")}</span> {t(`lr_source_${info.source}`)}</li>
        <li data-ok={info.exists}>
          <Icon name={info.exists ? "check" : "alert"} size={14} />{t(info.exists ? "lr_exists" : "lr_missing")}
        </li>
        <li data-ok={info.writable}>
          <Icon name={info.writable ? "check" : "alert"} size={14} />{t(info.writable ? "lr_writable" : "lr_not_writable")}
        </li>
      </ul>
    {:else}
      <p class="muted">{t("lr_none")}</p>
    {/if}

    {#if picker.step === "reviewing" || picker.step === "saving"}
      {@const summary = picker.step === "reviewing" ? picker.summary : null}
      {@const msg = summary ? summaryMessage(summary) : null}
      <div class="review" tabindex="-1" bind:this={review} role="group" aria-labelledby="lr-review-{uid}">
        <h3 id="lr-review-{uid}">{t("lr_review_title")}</h3>
        <code class="path chosen">{picker.path}</code>
        <p class="summary" aria-live="polite">
          {#if msg}
            <Icon name={msg.key === "lr_summary_has" ? "library" : "download"} size={16} />
            {t(msg.key, { platforms: msg.platforms })}
          {:else}
            {t("lr_checking")}
          {/if}
        </p>
        <p class="no-move"><Icon name="alert" size={16} />{t("lr_no_move")}</p>
        <div class="actions">
          <button type="button" class="btn btn-primary" onclick={use} disabled={picker.step === "saving"}>
            {t(picker.step === "saving" ? "lr_saving" : "lr_use")}
          </button>
          <button type="button" class="btn btn-quiet" onclick={cancel}>{t("dl_cancel")}</button>
        </div>
      </div>
    {:else if picker.step === "error"}
      <div class="error" role="alert">
        <Icon name="alert" size={18} />
        <div>
          <p class="what">{t("lr_err_title")}</p>
          {#if picker.path}<code class="path" title={picker.path}>{middleTruncate(picker.path, 56)}</code>{/if}
          <p class="next">{t(`lr_err_${picker.kind}`)}</p>
        </div>
        <button type="button" class="btn link" onclick={cancel}>{t("del_close")}</button>
      </div>
    {:else if picker.step === "picking"}
      <p class="muted" role="status">{t("lr_picking")}</p>
    {/if}

    {#if confirmingReset}
      <div class="confirm" role="group">
        <p>{t("lr_reset_confirm")}</p>
        <div class="actions">
          <button type="button" class="btn btn-quiet" onclick={reset}>{t("lr_reset_yes")}</button>
          <button type="button" class="btn link" onclick={() => (confirmingReset = false)}>{t("dl_cancel")}</button>
        </div>
      </div>
    {/if}

    {#if openFailed}<p class="muted" role="status">{t("lr_open_failed")}</p>{/if}

    <div class="actions">
      <button
        type="button"
        class="btn btn-quiet"
        bind:this={changeButton}
        onclick={change}
        disabled={picker.step === "picking" || picker.step === "saving"}
      >
        {t("lr_change")}
      </button>
      {#if info?.path && info.exists}
        <button type="button" class="btn link" onclick={openFolder}>{t("lr_open")}</button>
      {/if}
      {#if info?.source === "saved" && !confirmingReset}
        <button type="button" class="btn link" onclick={() => (confirmingReset = true)}>{t("lr_reset")}</button>
      {/if}
    </div>
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
    gap: var(--s-4);
    min-width: 0;
  }
  .path-row {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
  }
  .path {
    font-family: ui-monospace, "SFMono-Regular", "Cascadia Mono", "DejaVu Sans Mono", monospace;
    font-size: var(--fs-sm);
    color: var(--text);
    user-select: all;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
  .facts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-2) var(--s-6);
    font-size: var(--fs-sm);
    color: var(--text-muted);
  }
  .facts li {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
  }
  .facts [data-ok="true"] :global(svg) {
    color: var(--ok);
  }
  .facts [data-ok="false"] {
    color: var(--danger);
  }
  .label {
    color: var(--text-muted);
    margin-right: var(--s-1);
  }
  .facts li:first-child {
    color: var(--text);
  }
  .muted {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .review {
    display: grid;
    gap: var(--s-3);
    padding: var(--s-5);
    background: var(--surface-1);
    border: 1px solid var(--line);
    border-left: 2px solid var(--lumen);
    border-radius: var(--r-3);
  }
  .review h3 {
    font-size: var(--fs-lg);
  }
  .chosen {
    font-size: var(--fs-sm);
    white-space: normal;
    overflow-wrap: anywhere;
  }
  .summary,
  .no-move {
    display: flex;
    align-items: flex-start;
    gap: var(--s-2);
    font-size: var(--fs-sm);
  }
  .summary {
    color: var(--text);
  }
  .no-move {
    color: var(--text-muted);
  }
  .summary :global(svg),
  .no-move :global(svg) {
    flex: none;
    margin-top: 2px;
  }
  .no-move :global(svg) {
    color: var(--lumen);
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
  }
  .error > div {
    flex: 1;
    min-width: 0;
    display: grid;
    gap: 2px;
  }
  .what {
    font-size: var(--fs-sm);
    font-weight: 500;
  }
  .next {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .confirm {
    display: grid;
    gap: var(--s-2);
    font-size: var(--fs-sm);
    color: var(--text-muted);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
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
