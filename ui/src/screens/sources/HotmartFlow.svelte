<script lang="ts">
  // Hotmart flow inside Descargas: connect, list purchased courses, download.
  import { i18n, t } from "../../lib/i18n/index.svelte";
  import { formatIsoDate } from "../../lib/logic/time";
  import { app } from "../../lib/state/app.svelte";
  import { hotmart } from "../../lib/state/downloads.svelte";
  import ConnectPanel from "./ConnectPanel.svelte";
  import ErrorNote from "./ErrorNote.svelte";
  import CourseList from "./CourseList.svelte";
  import type { CourseListItem } from "./CourseList.svelte";
  import SessionBar from "./SessionBar.svelte";

  let token = $state("");
  let cookies = $state("");
  const uid = $props.id();

  const courses = $derived<CourseListItem[]>(hotmart.items.map((course) => {
    const date = formatIsoDate(course.purchaseDate, i18n.locale);
    return {
      key: course.id,
      title: course.name,
      coverUrl: course.coverUrl,
      facts: [...(course.seller ? [course.seller] : []), ...(date ? [t("hm_purchased", { date })] : [])],
      folder: course.folder,
    };
  }));

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    const tk = token.trim();
    if (!tk || !app.api) return;
    const api = app.api;
    const ck = cookies.trim();
    const ok = await hotmart.loginManual(async () => (await api.hotmartLoginCredentials(tk, ck || undefined)).courses);
    if (ok) {
      token = "";
      cookies = "";
    }
  }
</script>

{#snippet manual(busy: boolean)}
  <form onsubmit={submit}>
    <ol class="steps">
      <li><span class="n num">1</span>{t("hm_token_step1")}</li>
      <li><span class="n num">2</span>{t("hm_token_step2")}</li>
      <li><span class="n num">3</span>{t("hm_token_step3")}</li>
    </ol>
    <label for="token-{uid}" class="eyebrow">{t("hm_token_label")}</label>
    <input
      id="token-{uid}"
      type="password"
      bind:value={token}
      autocomplete="off"
      spellcheck="false"
      placeholder={t("hm_token_placeholder")}
      disabled={busy}
    />
    <details class="advanced">
      <summary>{t("hm_advanced")}</summary>
      <label for="cookies-{uid}" class="eyebrow">{t("hm_cookies_label")}</label>
      <textarea id="cookies-{uid}" bind:value={cookies} rows="3" autocomplete="off" spellcheck="false" disabled={busy}></textarea>
      <p class="hint">{t("hm_cookies_hint")}</p>
    </details>
    <div>
      <button type="submit" class="btn btn-quiet" disabled={!token.trim() || busy}>
        {t(hotmart.session === "connecting" ? "dl_connecting" : "dl_connect")}
      </button>
    </div>
  </form>
{/snippet}

<div class="flow">
  {#if hotmart.session === "unknown"}
    <p class="muted" role="status">{t("loading")}</p>
  {:else if hotmart.session === "connected"}
    <SessionBar session={hotmart} label={t("hm_connected")} confirmLabel={t("hm_logout_confirm")} />
    {#if hotmart.sessionError}<ErrorNote error={hotmart.sessionError} />{/if}
    <CourseList session={hotmart} platform="hotmart" items={courses} heading={t("hm_courses")} emptyText={t("hm_no_courses")} />
  {:else}
    <ConnectPanel
      session={hotmart}
      platformName="Hotmart"
      title={t("hm_connect_title")}
      body={t("hm_connect_body")}
      loginLabel={t("hm_login_webview")}
      manualLabel={t("hm_token_toggle")}
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
  .advanced {
    display: grid;
    gap: var(--s-2);
  }
  .advanced summary {
    cursor: pointer;
    color: var(--text-muted);
    font-size: var(--fs-sm);
    width: max-content;
  }
  .advanced summary:hover {
    color: var(--text);
  }
  .advanced[open] summary {
    margin-bottom: var(--s-2);
  }
  .hint {
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  @media (max-height: 640px) {
    .flow {
      gap: var(--s-5);
    }
  }
</style>
