<script lang="ts">
  // Native modal dialog (focus trapped, Esc closes) to delete a downloaded
  // group/course from this computer. Safe button first; permanent deletion
  // only behind a separate disclosure and an explicit checkbox.
  import Icon from "../../lib/design/Icon.svelte";
  import { i18n, t, tn } from "../../lib/i18n/index.svelte";
  import { formatBytes } from "../../lib/logic/deletion";
  import { platformById } from "../../lib/platforms";
  import { deletion } from "../../lib/state/deletion.svelte";

  let { onclosed }: { onclosed: () => void } = $props();
  let dialog: HTMLDialogElement | undefined = $state();
  let permanentOpen = $state(false);
  let understood = $state(false);
  let cancelButton: HTMLButtonElement | undefined = $state();
  const uid = $props.id();

  const s = $derived(deletion.state);
  const preview = $derived("preview" in s ? s.preview : null);
  const platformName = $derived(platformById(s.step !== "idle" ? s.target.platform : null)?.name ?? "");
  const error = $derived(s.step === "error" ? s.error : null);
  const running = $derived(!!preview?.runningDownload || error?.code === "download_running");
  const busy = $derived(s.step === "deleting");
  const trashBlocked = $derived(running || busy || !preview || error?.code === "trash_failed" || error?.code === "not_found");
  const permanentBlocked = $derived(running || busy || !preview || !understood || error?.code === "not_found");

  // Open on demand; close (and return focus) when the flow ends.
  $effect(() => {
    if (!dialog) return;
    if (s.step !== "idle" && s.step !== "done" && !dialog.open) {
      permanentOpen = false;
      understood = false;
      dialog.showModal();
      queueMicrotask(() => cancelButton?.focus());
    }
    if ((s.step === "idle" || s.step === "done") && dialog.open) dialog.close();
  });

  // A trash failure leads to the permanent path.
  $effect(() => {
    if (error?.code === "trash_failed") permanentOpen = true;
  });

  function onClose() {
    if (deletion.state.step !== "idle") deletion.close();
    onclosed();
  }
</script>

<dialog bind:this={dialog} class="dialog" aria-labelledby="del-h-{uid}" onclose={onClose}>
  {#if s.step !== "idle"}
    <h2 id="del-h-{uid}">{t("del_title", { name: s.title })}</h2>

    {#if s.step === "previewing"}
      <p class="muted" role="status">{t("del_loading")}</p>
    {:else if preview}
      <ul class="facts num">
        <li>{tn("del_lessons", preview.lessons)}</li>
        <li>{t("del_with_video", { n: preview.lessonsWithVideo })}</li>
        <li class="space">{t("del_space", { size: formatBytes(preview.bytes, preview.bytesApprox, i18n.locale) })}</li>
      </ul>
      <p class="where"><span class="eyebrow">{t("del_path")}</span><code title={preview.path}>{preview.path}</code></p>
      <p class="reassure"><Icon name="check" size={16} />{t("del_reassure", { platform: platformName })}</p>
    {/if}

    {#if running}
      <p class="note" role="status"><Icon name="alert" size={16} />{t("del_running", { platform: platformName })}</p>
    {:else if error}
      <p class="note error" role="alert"><Icon name="alert" size={16} />{t(`del_err_${error.code}`, { platform: platformName })}</p>
    {/if}

    <div class="actions">
      <button type="button" class="btn btn-quiet" bind:this={cancelButton} onclick={() => dialog?.close()}>
        {t(error && !preview ? "del_close" : "del_cancel")}
      </button>
      {#if preview}
        <button type="button" class="btn btn-quiet trash" disabled={trashBlocked} onclick={() => deletion.confirm(false)}>
          <Icon name="trash" size={16} />{t(busy && s.step === "deleting" && !s.permanent ? "del_deleting" : "del_trash")}
        </button>
      {/if}
    </div>

    {#if preview}
      <div class="permanent">
        <button
          type="button"
          class="disclosure"
          aria-expanded={permanentOpen}
          aria-controls="perm-{uid}"
          onclick={() => (permanentOpen = !permanentOpen)}
        >
          <Icon name="chevronDown" size={16} />{t("del_permanent_toggle")}
        </button>
        <div id="perm-{uid}" class="perm-body" hidden={!permanentOpen}>
          <p class="muted">{t("del_permanent_body")}</p>
          <label class="check">
            <input type="checkbox" bind:checked={understood} disabled={running || busy} />
            <span>{t("del_permanent_check", { name: s.title })}</span>
          </label>
          <div>
            <button type="button" class="btn btn-quiet danger" disabled={permanentBlocked} onclick={() => deletion.confirm(true)}>
              {t(busy && s.step === "deleting" && s.permanent ? "del_deleting" : "del_permanent_button")}
            </button>
          </div>
        </div>
      </div>
    {/if}
  {/if}
</dialog>

<style>
  .dialog {
    width: min(560px, calc(100vw - 32px));
    max-height: calc(100vh - 32px);
    padding: var(--s-8);
    background: var(--surface-1);
    color: var(--text);
    border: 1px solid var(--line);
    border-radius: var(--r-3);
    overflow-y: auto;
  }
  .dialog::backdrop {
    background: color-mix(in srgb, var(--bg) 78%, transparent);
  }
  .dialog[open] {
    display: grid;
    gap: var(--s-5);
  }
  h2 {
    font-size: var(--fs-lg);
    line-height: var(--lh-snug);
    padding-right: var(--s-4);
  }
  .facts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-2) var(--s-5);
    font-size: var(--fs-sm);
    color: var(--text-muted);
  }
  .space {
    color: var(--text);
  }
  .where {
    display: grid;
    gap: var(--s-1);
    min-width: 0;
  }
  .where .eyebrow {
    font-family: var(--font-ui);
  }
  code {
    font-family: ui-monospace, "SFMono-Regular", "Cascadia Mono", "DejaVu Sans Mono", monospace;
    font-size: var(--fs-xs);
    color: var(--text);
    overflow-wrap: anywhere;
    user-select: all;
  }
  .reassure,
  .note {
    display: flex;
    align-items: flex-start;
    gap: var(--s-2);
    font-size: var(--fs-sm);
  }
  .reassure {
    color: var(--text);
  }
  .reassure :global(svg) {
    color: var(--ok);
    flex: none;
    margin-top: 2px;
  }
  .note {
    color: var(--text-muted);
    padding: var(--s-3) var(--s-4);
    background: var(--surface-2);
    border-left: 2px solid var(--moon);
    border-radius: 0 var(--r-2) var(--r-2) 0;
  }
  .note.error {
    border-left-color: var(--danger);
  }
  .note :global(svg) {
    flex: none;
    margin-top: 2px;
  }
  .muted {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-2);
    justify-content: flex-end;
  }
  .trash:not(:disabled) {
    border-color: var(--text-muted);
  }
  .permanent {
    border-top: 1px solid var(--line);
    padding-top: var(--s-3);
  }
  .disclosure {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    height: 32px;
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .disclosure:hover {
    color: var(--text);
  }
  .disclosure :global(svg) {
    transition: transform var(--dur-2) var(--ease-out);
  }
  .disclosure[aria-expanded="true"] :global(svg) {
    transform: rotate(180deg);
  }
  .perm-body {
    display: grid;
    gap: var(--s-3);
    padding-top: var(--s-2);
  }
  .perm-body[hidden] {
    display: none;
  }
  .check {
    display: flex;
    align-items: flex-start;
    gap: var(--s-2);
    font-size: var(--fs-sm);
    color: var(--text);
    cursor: pointer;
  }
  .check input {
    margin-top: 3px;
    accent-color: var(--danger);
  }
  .danger:not(:disabled) {
    border-color: var(--danger);
    color: var(--danger);
  }
</style>
