<script lang="ts">
  import Icon, { type IconName } from "../design/Icon.svelte";
  import { t, type MessageKey } from "../i18n/index.svelte";
  import { app } from "../state/app.svelte";
  import { router } from "../state/router.svelte";
  import BrandMark from "./BrandMark.svelte";

  type Item = { id: "home" | "library" | "downloads" | "settings"; icon: IconName; label: MessageKey; href: string };
  const items: Item[] = [
    { id: "home", icon: "home", label: "nav_home", href: "#/" },
    { id: "library", icon: "library", label: "nav_library", href: "#/library" },
    { id: "downloads", icon: "download", label: "nav_downloads", href: "#/downloads" },
    { id: "settings", icon: "settings", label: "nav_settings", href: "#/settings" },
  ];

  const section = $derived.by(() => {
    const n = router.route.name;
    return n === "course" || n === "watch" ? "library" : n;
  });
</script>

<nav class="rail" aria-label={t("nav_main")}>
  <a class="brand" href="#/" aria-label="Lumen">
    <BrandMark size={28} />
    <span class="wordmark">Lumen</span>
  </a>
  <ul>
    {#each items as item (item.id)}
      <li>
        <a
          class="item"
          href={item.href}
          aria-current={section === item.id ? "page" : undefined}
          title={t(item.label)}
        >
          <Icon name={item.icon} />
          <span class="label">{t(item.label)}</span>
        </a>
      </li>
    {/each}
  </ul>
  <div class="foot">
    {#if app.mode === "mock"}
      <span class="eyebrow preview">{t("preview_mode")}</span>
    {:else}
      <span class="version num">v{app.version}</span>
    {/if}
  </div>
</nav>

<style>
  .rail {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--s-8);
    width: var(--rail-w);
    padding: var(--s-6) var(--s-3);
    border-right: 1px solid var(--line);
    background: var(--bg);
  }
  .brand {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    padding: 0 var(--s-3);
    color: var(--text);
    text-decoration: none;
    height: 32px;
  }
  .wordmark {
    font-family: var(--font-display);
    font-size: 1.3125rem;
    font-weight: var(--fw-display);
    letter-spacing: -0.02em;
    line-height: 1;
    padding-top: 2px;
  }
  ul {
    display: grid;
    gap: var(--s-1);
  }
  .item {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--s-3);
    height: 40px;
    padding: 0 var(--s-3);
    border-radius: var(--r-2);
    color: var(--text-muted);
    text-decoration: none;
    font-size: var(--fs-sm);
    font-weight: 500;
    transition:
      color var(--dur-1) var(--ease-out),
      background-color var(--dur-1) var(--ease-out);
  }
  .item:hover {
    color: var(--text);
    background: var(--surface-1);
  }
  .item[aria-current="page"] {
    color: var(--text);
  }
  .item[aria-current="page"]::before {
    content: "";
    position: absolute;
    left: calc(-1 * var(--s-3));
    top: 10px;
    bottom: 10px;
    width: 2px;
    background: var(--lumen);
  }
  .foot {
    margin-top: auto;
    display: grid;
    gap: var(--s-1);
    padding: 0 var(--s-3);
  }
  .version {
    font-size: var(--fs-2xs);
    color: var(--text-muted);
  }
  .preview {
    color: var(--moon);
  }

  @media (max-width: 900px) {
    .rail {
      padding-inline: var(--s-2);
      align-items: center;
    }
    .brand {
      padding: 0;
    }
    .wordmark,
    .label,
    .foot {
      display: none;
    }
    .item {
      width: 44px;
      justify-content: center;
      padding: 0;
    }
    .item[aria-current="page"]::before {
      left: -10px;
    }
  }
</style>
