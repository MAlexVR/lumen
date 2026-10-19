<script lang="ts">
  // Skool flow inside Descargas: connect, list communities, download.
  import { t, tn } from "../../lib/i18n/index.svelte";
  import { groupFolder, localStatus } from "../../lib/logic/localStatus";
  import { app } from "../../lib/state/app.svelte";
  import { skool } from "../../lib/state/downloads.svelte";
  import ConnectPanel from "./ConnectPanel.svelte";
  import ErrorNote from "./ErrorNote.svelte";
  import ItemRow from "./ItemRow.svelte";
  import SessionBar from "./SessionBar.svelte";

  let cookie = $state("");
  const uid = $props.id();

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    const text = cookie.trim();
    if (!text || !app.api) return;
    const api = app.api;
    if (await skool.loginManual(async () => (await api.skoolLoginCookies(text)).groups)) cookie = "";
  }
</script>

{#snippet manual(busy: boolean)}
  <form onsubmit={submit}>
    <ol class="steps">
      <li><span class="n num">1</span>{t("dl_cookies_step1")}</li>
      <li><span class="n num">2</span>{t("dl_cookies_step2")}</li>
      <li><span class="n num">3</span>{t("dl_cookies_step3")}</li>
    </ol>
    <label for="cookie-{uid}" class="eyebrow">{t("dl_cookies_label")}</label>
    <textarea
      id="cookie-{uid}"
      bind:value={cookie}
      rows="3"
      autocomplete="off"
      spellcheck="false"
      placeholder={t("dl_cookies_placeholder")}
      disabled={busy}
    ></textarea>
    <div>
      <button type="submit" class="btn btn-quiet" disabled={!cookie.trim() || busy}>
        {t(skool.session === "connecting" ? "dl_connecting" : "dl_connect")}
      </button>
    </div>
  </form>
{/snippet}

<div class="flow">
  {#if skool.session === "unknown"}
    <p class="muted" role="status">{t("loading")}</p>
  {:else if skool.session === "connected"}
    <SessionBar session={skool} label={t("dl_connected")} />
    {#if skool.sessionError}<ErrorNote error={skool.sessionError} />{/if}
    <section aria-labelledby="sk-items-h">
      <h2 id="sk-items-h" class="eyebrow section">{t("dl_groups")}</h2>
      {#if skool.items.length}
        <ul class="items">
          {#each skool.items as group (group.slug)}
            <ItemRow
              session={skool}
              platform="skool"
              itemKey={group.slug}
              name={group.name}
              coverUrl={group.coverUrl}
              facts={[tn("courses", group.numCourses)]}
              local={localStatus(group, app.library)}
              folder={groupFolder(group.name)}
            />
          {/each}
        </ul>
      {:else}
        <p class="muted">{t("dl_no_groups")}</p>
      {/if}
    </section>
  {:else}
    <ConnectPanel
      session={skool}
      platformName="Skool"
      title={t("dl_connect_title")}
      body={t("dl_connect_body")}
      loginLabel={t("dl_login_webview")}
      manualLabel={t("dl_cookies_toggle")}
      {manual}
    />
  {/if}
</div>

<style>
  .flow {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: var(--s-8);
  }
  .muted {
    color: var(--text-muted);
  }
  .section {
    font-family: var(--font-ui);
    margin-bottom: var(--s-2);
  }
  .items {
    border-top: 1px solid var(--line);
  }
  @media (max-height: 640px) {
    .flow {
      gap: var(--s-5);
    }
  }
</style>
