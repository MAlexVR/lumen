<script lang="ts" generics="Item">
  // Connected state header with a confirmed sign-out.
  import { t } from "../../lib/i18n/index.svelte";
  import type { PlatformSession } from "../../lib/state/downloads.svelte";

  let { session, label, confirmLabel = t("dl_logout_confirm") }: { session: PlatformSession<Item>; label: string; confirmLabel?: string } = $props();
  let confirming = $state(false);
  let heading: HTMLElement | undefined = $state();
  // svelte-ignore state_referenced_locally
  let previous = session.session;

  // After a successful login, focus the connected heading.
  $effect(() => {
    const now = session.session;
    if (now === "connected" && (previous === "waiting" || previous === "connecting")) heading?.focus();
    previous = now;
  });

  async function logout() {
    confirming = false;
    await session.logout();
  }
</script>

<div class="status">
  <span class="ok-dot" aria-hidden="true"></span>
  <h2 tabindex="-1" bind:this={heading}>{label}</h2>
  <span class="spacer"></span>
  {#if confirming}
    <p class="confirm" role="alert">{confirmLabel}</p>
    <div class="confirm-actions">
      <button type="button" class="btn btn-quiet danger" onclick={logout}>{t("dl_logout_yes")}</button>
      <button type="button" class="btn link" onclick={() => (confirming = false)}>{t("dl_cancel")}</button>
    </div>
  {:else}
    <button type="button" class="btn link" onclick={() => (confirming = true)}>{t("dl_logout")}</button>
  {/if}
</div>

<style>
  .status {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-3);
    padding: var(--s-4) 0;
    border-top: 1px solid var(--line);
    border-bottom: 1px solid var(--line);
  }
  h2 {
    font-family: var(--font-ui);
    font-size: var(--fs-sm);
    font-weight: 500;
    letter-spacing: 0.01em;
  }
  .ok-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--ok);
  }
  .spacer {
    flex: 1;
  }
  .confirm {
    color: var(--text-muted);
    font-size: var(--fs-sm);
    flex-basis: 100%;
    order: 5;
  }
  .confirm-actions {
    display: flex;
    gap: var(--s-2);
  }
  .danger {
    border-color: var(--danger);
    color: var(--danger);
  }
  .link {
    color: var(--text-muted);
    padding: 0 var(--s-3);
  }
  .link:hover {
    color: var(--text);
  }
</style>
