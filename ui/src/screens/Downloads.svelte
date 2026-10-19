<script lang="ts">
  // Descargas: platform picker; a platform id opens its flow or, when the
  // platform is not in Lumen yet, a calm inline detail.
  import Icon from "../lib/design/Icon.svelte";
  import { t } from "../lib/i18n/index.svelte";
  import { platformById } from "../lib/platforms";
  import { href } from "../lib/state/router.svelte";
  import PlatformGrid from "./platforms/PlatformGrid.svelte";
  import PlatformSoon from "./platforms/PlatformSoon.svelte";
  import PlatformStage from "./platforms/PlatformStage.svelte";
  import HotmartFlow from "./sources/HotmartFlow.svelte";
  import PluralsightFlow from "./sources/PluralsightFlow.svelte";
  import SkoolFlow from "./sources/SkoolFlow.svelte";
  import UdemyFlow from "./sources/UdemyFlow.svelte";
  import YoutubeFlow from "./sources/YoutubeFlow.svelte";

  let { platform: platformId }: { platform: string | null } = $props();
  const platform = $derived(platformById(platformId));

  // The tile to focus when returning to the picker.
  let returnTo = $state<string | null>(null);
  $effect(() => {
    if (platform) returnTo = platform.id;
  });
</script>

<div class="page">
  {#if platform}
    <a class="back" href={href({ name: "downloads", platform: null })}>
      <Icon name="arrowLeft" size={16} />{t("pf_all")}
    </a>
  {/if}

  {#if platform?.status === "available" && (platform.id === "skool" || platform.id === "hotmart" || platform.id === "pluralsight" || platform.id === "youtube" || platform.id === "udemy")}
    <header class="head flow-head">
      <span class="logo"><PlatformStage {platform} alt={t("pf_logo", { name: platform.name })} /></span>
      <div>
        <span class="eyebrow">{t("downloads_title")}</span>
        <h1>{platform.name}</h1>
        <p class="lede">{t(platform.id === "skool" ? "dl_lede" : platform.id === "hotmart" ? "hm_lede" : platform.id === "pluralsight" ? "ps_lede" : platform.id === "udemy" ? "ud_lede" : "yt_lede")}</p>
      </div>
    </header>
    {#if platform.id === "skool"}<SkoolFlow />{:else if platform.id === "hotmart"}<HotmartFlow />{:else if platform.id === "pluralsight"}<PluralsightFlow />{:else if platform.id === "udemy"}<UdemyFlow />{:else}<YoutubeFlow />{/if}
  {:else if platform}
    <PlatformSoon {platform} />
  {:else}
    <header class="head">
      <h1>{t("downloads_title")}</h1>
      <p class="lede">{t("pf_lede")}</p>
    </header>
    <PlatformGrid status="available" focusId={returnTo} />
    <PlatformGrid status="soon" focusId={returnTo} />
  {/if}
</div>

<style>
  .page {
    max-width: 1080px;
    padding: var(--s-12) var(--gutter) var(--s-20);
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: var(--s-10);
  }
  .back {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    justify-self: start;
    color: var(--text-muted);
    text-decoration: none;
    font-size: var(--fs-sm);
    margin-bottom: calc(-1 * var(--s-4));
  }
  .back:hover {
    color: var(--text);
  }
  .head {
    display: grid;
    gap: var(--s-3);
  }
  h1 {
    font-size: var(--fs-display);
    line-height: 1.04;
  }
  .lede {
    color: var(--text-muted);
    font-size: var(--fs-md);
    max-width: 56ch;
  }
  .flow-head {
    grid-template-columns: 112px minmax(0, 1fr);
    gap: var(--s-6);
    align-items: center;
  }
  .flow-head h1 {
    font-size: var(--fs-2xl);
  }
  .flow-head .lede {
    font-size: var(--fs-base);
  }
  .logo :global(.stage) {
    outline: 1px solid var(--line);
    outline-offset: 3px;
  }
  @media (max-height: 640px) {
    .page {
      padding-top: var(--s-6);
      gap: var(--s-6);
    }
    h1 {
      font-size: var(--fs-2xl);
    }
    .flow-head h1 {
      font-size: var(--fs-xl);
    }
  }
</style>
