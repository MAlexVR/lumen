<script lang="ts">
  // A typed Skool error: what happened and what to do next.
  import type { Snippet } from "svelte";
  import Icon from "../../lib/design/Icon.svelte";
  import { t, type MessageKey } from "../../lib/i18n/index.svelte";
  import type { DownloadError, PlatformId } from "../../lib/types";

  let { error, platform = null, children }: { error: DownloadError; platform?: PlatformId | null; children?: Snippet } = $props();
  // Pluralsight words the session errors for itself; every other code is shared.
  const PS_CODES = ["not_logged_in", "session_expired", "invalid_cookies", "already_running", "no_library", "login_cancelled", "login_timeout", "network", "invalid_token", "internal"];
  // Golden Life signs in with email and password, so it words its own session errors too.
  const GL_CODES = ["not_logged_in", "session_expired", "invalid_credentials", "already_running", "no_library", "network", "internal"];
  // Harbiz signs in the same way and words its errors for itself.
  const HB_CODES = GL_CODES;
  const prefix = $derived(
    platform === "pluralsight" && PS_CODES.includes(error.code)
      ? "ps_err"
      : platform === "goldenlife" && GL_CODES.includes(error.code)
        ? "gl_err"
        : platform === "harbiz" && HB_CODES.includes(error.code)
          ? "hb_err"
          : "err",
  );
</script>

<div class="note" role="alert" data-code={error.code}>
  <Icon name="alert" size={18} />
  <div class="text">
    <p class="what">{t(`${prefix}_${error.code}` as MessageKey)}</p>
    <p class="next">{t(`${prefix}_${error.code}_hint` as MessageKey)}</p>
  </div>
  {#if children}<div class="action">{@render children()}</div>{/if}
</div>

<style>
  .note {
    display: flex;
    align-items: flex-start;
    gap: var(--s-3);
    padding: var(--s-3) var(--s-4);
    border-left: 2px solid var(--danger);
    background: var(--surface-1);
    border-radius: 0 var(--r-2) var(--r-2) 0;
  }
  .note > :global(svg) {
    color: var(--danger);
    margin-top: 2px;
  }
  .text {
    flex: 1;
    min-width: 0;
  }
  .what {
    color: var(--text);
    font-size: var(--fs-sm);
    font-weight: 500;
  }
  .next {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .action {
    align-self: center;
  }
</style>
