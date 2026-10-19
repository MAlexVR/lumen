<script lang="ts">
  import Rail from "./lib/components/Rail.svelte";
  import { t } from "./lib/i18n/index.svelte";
  import { app } from "./lib/state/app.svelte";
  import { thumbs } from "./lib/state/thumbs.svelte";
  import { initDownloads } from "./lib/state/downloads.svelte";
  import { router } from "./lib/state/router.svelte";
  import Course from "./screens/Course.svelte";
  import Downloads from "./screens/Downloads.svelte";
  import Home from "./screens/Home.svelte";
  import Library from "./screens/Library.svelte";
  import Player from "./screens/Player.svelte";
  import Settings from "./screens/Settings.svelte";

  let main: HTMLElement | undefined = $state();

  $effect(() => {
    void app.init().then(() => {
      if (app.api) void initDownloads(app.api);
      void thumbs.init();
    });
  });

  // New screen: start at the top.
  $effect(() => {
    void router.route;
    main?.scrollTo({ top: 0 });
  });
</script>

<div class="shell">
  <Rail />
  <main bind:this={main} class:flush={router.route.name === "watch"}>
    {#if app.status === "loading"}
      <p class="state" role="status">{t("loading")}</p>
    {:else if app.status === "error"}
      <p class="state error" role="alert">{t("load_error", { e: app.error })}</p>
    {:else if router.route.name === "settings"}
      <Settings />
    {:else if router.route.name === "downloads"}
      <Downloads platform={router.route.platform} />
    {:else if !app.groups.length}
      <div class="state">
        <h1>{t("empty_library")}</h1>
        <p>{t("empty_hint")}</p>
        <a href="#/settings">{t("nav_settings")}</a>
      </div>
    {:else if router.route.name === "library"}
      <Library platform={router.route.platform} />
    {:else if router.route.name === "course"}
      {#key `${router.route.group}/${router.route.course}`}
        <Course group={router.route.group} course={router.route.course} />
      {/key}
    {:else if router.route.name === "watch"}
      <Player group={router.route.group} lessonKey={router.route.key} />
    {:else}
      <Home />
    {/if}
  </main>
</div>

<style>
  .shell {
    display: flex;
    height: 100vh;
    min-width: 0;
  }
  main {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    overflow-x: hidden;
    border-top: 1px solid var(--line);
    scroll-behavior: smooth;
  }
  @media (prefers-reduced-motion: reduce) {
    main {
      scroll-behavior: auto;
    }
  }
  .state {
    padding: var(--s-20) var(--gutter);
    color: var(--text-muted);
    display: grid;
    gap: var(--s-4);
    max-width: 680px;
  }
  .state h1 {
    color: var(--text);
    font-size: var(--fs-xl);
  }
  .error {
    color: var(--danger);
  }
</style>
