<script lang="ts" generics="Item">
  // Not connected: in-app login window (primary) or a manual form
  // (Skool cookie header, Hotmart token) in a collapsed disclosure.
  import type { Snippet } from "svelte";
  import Icon from "../../lib/design/Icon.svelte";
  import BrandMark from "../../lib/components/BrandMark.svelte";
  import { t } from "../../lib/i18n/index.svelte";
  import { formatTime } from "../../lib/logic/time";
  import { LOGIN_TIMEOUT_MS, type PlatformSession } from "../../lib/state/downloads.svelte";
  import ErrorNote from "./ErrorNote.svelte";

  interface Props {
    session: PlatformSession<Item>;
    platformName: string;
    title: string;
    body: string;
    loginLabel: string;
    manualLabel: string;
    errorPlatform?: import("../../lib/types").PlatformId;
    onWebviewLogin?: () => Promise<boolean>;
    /** The manual form; receives whether a login is in progress. */
    manual: Snippet<[boolean]>;
  }
  let { session, platformName, title, body, loginLabel, manualLabel, errorPlatform, onWebviewLogin, manual }: Props = $props();

  let manualOpen = $state(false);
  let now = $state(Date.now());
  const uid = $props.id();

  const waiting = $derived(session.session === "waiting");
  const busy = $derived(session.session === "connecting" || waiting);
  const left = $derived(
    session.loginStartedAt ? Math.max(0, LOGIN_TIMEOUT_MS - (now - session.loginStartedAt)) / 1000 : 0,
  );

  $effect(() => {
    if (!waiting) return;
    const timer = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(timer);
  });

  // A timed-out or malformed login suggests the manual route.
  $effect(() => {
    const code = session.sessionError?.code;
    if (code === "login_timeout" || code === "invalid_token" || code === "invalid_cookies") manualOpen = true;
  });
</script>

<section class="connect" aria-labelledby="connect-h-{uid}">
  <span class="halo" aria-hidden="true"></span>
  <div class="lead">
    <BrandMark size={36} />
    <h2 id="connect-h-{uid}">{title}</h2>
    <p class="body">{body}</p>

    {#if waiting}
      <div class="waiting" role="status" aria-live="polite">
        <span class="pulse" aria-hidden="true"></span>
        <div>
          <p class="strong">{t("dl_waiting_title")}</p>
          <p class="muted">{t("dl_waiting_body", { name: platformName })}</p>
          <p class="muted">{t("dl_waiting_left")} <span class="num time">{formatTime(left)}</span></p>
        </div>
      </div>
    {:else}
      <div class="actions">
        <button type="button" class="btn btn-primary" onclick={() => onWebviewLogin ? onWebviewLogin() : session.loginWebview()} disabled={busy}>
          <Icon name="external" size={16} />{loginLabel}
        </button>
      </div>
    {/if}

    {#if session.sessionError && !waiting}
      <ErrorNote error={session.sessionError} platform={errorPlatform} />
    {/if}
  </div>

  <div class="manual">
    <button
      type="button"
      class="disclosure"
      aria-expanded={manualOpen}
      aria-controls="manual-{uid}"
      onclick={() => (manualOpen = !manualOpen)}
    >
      <Icon name="chevronDown" size={16} />{manualLabel}
    </button>
    <div id="manual-{uid}" class="manual-body" hidden={!manualOpen}>
      {@render manual(busy)}
    </div>
  </div>
</section>

<style>
  .connect {
    position: relative;
    display: grid;
    gap: var(--s-10);
    max-width: 760px;
    padding: var(--s-6) 0 var(--s-4);
  }
  .halo {
    position: absolute;
    left: -10%;
    top: -12%;
    width: 60%;
    height: 70%;
    background: radial-gradient(closest-side, var(--lumen-glow), transparent 72%);
    pointer-events: none;
  }
  .lead {
    position: relative;
    display: grid;
    gap: var(--s-5);
    justify-items: start;
  }
  h2 {
    font-size: var(--fs-2xl);
  }
  .body {
    color: var(--text-muted);
    font-size: var(--fs-md);
    max-width: 52ch;
  }
  .actions {
    margin-top: var(--s-2);
  }
  .waiting {
    display: flex;
    gap: var(--s-4);
    align-items: flex-start;
    padding: var(--s-4) var(--s-5);
    border: 1px solid var(--line);
    border-radius: var(--r-3);
    background: var(--surface-1);
  }
  .pulse {
    flex: none;
    width: 10px;
    height: 10px;
    margin-top: 6px;
    border-radius: 50%;
    background: var(--lumen);
    animation: pulse 1.6s var(--ease-out) infinite;
  }
  @keyframes pulse {
    0% {
      box-shadow: 0 0 0 0 var(--lumen-glow);
    }
    100% {
      box-shadow: 0 0 0 14px transparent;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .pulse {
      animation: none;
    }
  }
  .strong {
    color: var(--text);
  }
  .muted {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .time {
    color: var(--lumen);
    margin-left: var(--s-1);
  }
  .manual {
    position: relative;
    border-top: 1px solid var(--line);
    padding-top: var(--s-4);
  }
  .disclosure {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    color: var(--text-muted);
    font-size: var(--fs-sm);
    height: 36px;
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
  .manual-body {
    padding-top: var(--s-3);
  }
  .manual-body[hidden] {
    display: none;
  }
  /* Shared look of the manual forms rendered through the snippet. */
  .manual-body :global(form) {
    display: grid;
    gap: var(--s-3);
  }
  .manual-body :global(.steps) {
    display: grid;
    gap: var(--s-2);
    margin-bottom: var(--s-2);
  }
  .manual-body :global(.steps li) {
    display: grid;
    grid-template-columns: 24px 1fr;
    gap: var(--s-2);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .manual-body :global(.steps .n) {
    font-family: var(--font-display);
    font-size: var(--fs-base);
    font-weight: var(--fw-numeral);
    color: var(--text);
    line-height: 1.4;
  }
  .manual-body :global(code) {
    color: var(--text);
    font-size: 0.95em;
  }
  .manual-body :global(label) {
    font-family: var(--font-ui);
  }
  .manual-body :global(textarea),
  .manual-body :global(input[type="password"]) {
    width: 100%;
    padding: var(--s-3);
    background: var(--surface-1);
    color: var(--text);
    border: 1px solid var(--line);
    border-radius: var(--r-2);
    font: var(--fs-sm) / 1.5 var(--font-ui);
  }
  .manual-body :global(textarea) {
    resize: vertical;
    min-height: 76px;
  }
  .manual-body :global(textarea:focus-visible),
  .manual-body :global(input[type="password"]:focus-visible) {
    border-color: var(--lumen);
    outline: none;
  }
  .manual-body :global(textarea::placeholder),
  .manual-body :global(input::placeholder) {
    color: var(--text-muted);
  }
  @media (max-height: 640px) {
    .connect {
      padding-top: var(--s-2);
      gap: var(--s-6);
    }
    .lead {
      gap: var(--s-3);
    }
    h2 {
      font-size: var(--fs-xl);
    }
  }
</style>
