<script lang="ts">
  import { i18n, LOCALES, setLocale, t, type Locale } from "../lib/i18n/index.svelte";
  import { app } from "../lib/state/app.svelte";
  import LibraryRoot from "./settings/LibraryRoot.svelte";
  import Updates from "./settings/Updates.svelte";

  const NAMES: Record<Locale, string> = { es: "Español", en: "English" };
</script>

<div class="page">
  <header>
    <h1>{t("settings_title")}</h1>
  </header>

  <section class="row" aria-labelledby="set-lang">
    <h2 id="set-lang" class="eyebrow">{t("settings_language")}</h2>
    <div class="segmented" role="radiogroup" aria-labelledby="set-lang">
      {#each LOCALES as loc (loc)}
        <button
          type="button"
          role="radio"
          aria-checked={i18n.locale === loc}
          onclick={() => setLocale(loc)}
          lang={loc}
        >
          {NAMES[loc]}
        </button>
      {/each}
    </div>
  </section>

  <LibraryRoot />

  <Updates />

  <section class="row" aria-labelledby="set-about">
    <h2 id="set-about" class="eyebrow">{t("settings_about")}</h2>
    <p class="hint">Lumen <span class="num">{app.version}</span></p>
  </section>
</div>

<style>
  .page {
    max-width: 880px;
    padding: var(--s-16) var(--gutter);
  }
  h1 {
    font-size: var(--fs-2xl);
    margin-bottom: var(--s-12);
  }
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
  .segmented {
    display: inline-flex;
    border: 1px solid var(--line);
    border-radius: var(--r-2);
    padding: 2px;
    justify-self: start;
  }
  .segmented button {
    height: 32px;
    padding: 0 var(--s-4);
    border-radius: var(--r-1);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .segmented button[aria-checked="true"] {
    background: var(--surface-3);
    color: var(--text);
  }
  .hint {
    color: var(--text-muted);
    font-size: var(--fs-sm);
    margin-top: var(--s-2);
    max-width: 52ch;
  }
  @media (max-width: 760px) {
    .row {
      grid-template-columns: 1fr;
      gap: var(--s-3);
    }
  }
</style>
