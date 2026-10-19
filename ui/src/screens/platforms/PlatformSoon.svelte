<script lang="ts">
  // Calm inline detail for a platform that is not in Lumen yet.
  import { t } from "../../lib/i18n/index.svelte";
  import type { Platform } from "../../lib/platforms";
  import PlatformStage from "./PlatformStage.svelte";

  let { platform }: { platform: Platform } = $props();
  let heading: HTMLElement | undefined = $state();

  $effect(() => {
    void platform.id;
    heading?.focus();
  });
</script>

<section class="soon" aria-labelledby="soon-h">
  <div class="art">
    <PlatformStage {platform} alt={t("pf_logo", { name: platform.name })} />
  </div>
  <div class="text">
    <span class="status"><span class="marker" aria-hidden="true"></span>{t("pf_soon")}</span>
    <h2 id="soon-h" tabindex="-1" bind:this={heading}>{platform.name}</h2>
    <p class="lead">{t("pf_soon_title")}</p>
    <p class="body">{t("pf_soon_body", { name: platform.name })}</p>
  </div>
</section>

<style>
  .soon {
    display: grid;
    grid-template-columns: minmax(200px, 320px) minmax(0, 1fr);
    gap: var(--s-10);
    align-items: center;
    padding: var(--s-8) 0;
    border-top: 1px solid var(--line);
    border-bottom: 1px solid var(--line);
  }
  .art :global(.stage) {
    outline: 1px solid var(--line);
    outline-offset: 3px;
  }
  .text {
    display: grid;
    gap: var(--s-3);
    justify-items: start;
  }
  .status {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    font-size: var(--fs-2xs);
    font-weight: 500;
    letter-spacing: var(--tracking-label);
    text-transform: uppercase;
    color: var(--text-muted);
  }
  .marker {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    border: 1px solid var(--text-muted);
  }
  h2 {
    font-size: var(--fs-2xl);
  }
  .lead {
    font-size: var(--fs-md);
    color: var(--text);
  }
  .body {
    color: var(--text-muted);
    max-width: 52ch;
  }
  @media (max-width: 760px) {
    .soon {
      grid-template-columns: minmax(0, 1fr);
      gap: var(--s-6);
    }
    .art {
      max-width: 240px;
    }
  }
</style>
