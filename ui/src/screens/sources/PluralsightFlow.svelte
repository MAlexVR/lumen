<script lang="ts">
  // Pluralsight history course flow; credentials exist only in this component state.
  import { t } from "../../lib/i18n/index.svelte";
  import { displayPluralsightCourses } from "../../lib/logic/pluralsight";
  import { app } from "../../lib/state/app.svelte";
  import { pluralsight } from "../../lib/state/downloads.svelte";
  import ConnectPanel from "./ConnectPanel.svelte";
  import CourseList from "./CourseList.svelte";
  import type { CourseListItem } from "./CourseList.svelte";
  import ErrorNote from "./ErrorNote.svelte";
  import SessionBar from "./SessionBar.svelte";

  let cookies = $state("");
  const courses = $derived<CourseListItem[]>(displayPluralsightCourses(pluralsight.items, t("ps_course_missing_id")));

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    const text = cookies.trim();
    if (!text || !app.api) return;
    const api = app.api;
    await pluralsight.loginManual(async () => (await api.pluralsightLoginCredentials(text)).courses);
    cookies = "";
  }

  async function loginWebview() {
    cookies = "";
    try {
      return await pluralsight.loginWebview();
    } finally {
      cookies = "";
    }
  }

  async function openHistory() {
    await app.api?.openUrl("https://app.pluralsight.com/").catch(() => {});
  }
</script>

{#snippet manual(busy: boolean)}
  <form onsubmit={submit}>
    <ol class="steps">
      <li><span class="n num">1</span>{t("ps_cookie_step1")}</li>
      <li><span class="n num">2</span>{t("ps_cookie_step2")}</li>
      <li><span class="n num">3</span>{t("ps_cookie_step3")}</li>
    </ol>
    <label for="pluralsight-cookies" class="eyebrow">{t("ps_cookie_label")}</label>
    <textarea
      id="pluralsight-cookies"
      bind:value={cookies}
      rows="4"
      autocomplete="off"
      spellcheck="false"
      placeholder={t("ps_cookie_placeholder")}
      disabled={busy}
    ></textarea>
    <p class="hint">{t("ps_cookie_privacy")}</p>
    <button type="submit" class="btn btn-quiet" disabled={!cookies.trim() || busy}>
      {t(pluralsight.session === "connecting" ? "dl_connecting" : "dl_connect")}
    </button>
  </form>
{/snippet}

<div class="flow">
  {#if pluralsight.session === "unknown"}
    <p class="muted" role="status">{t("loading")}</p>
  {:else if pluralsight.session === "connected"}
    <SessionBar session={pluralsight} label={t("ps_connected")} confirmLabel={t("ps_logout_confirm")} />
    {#if pluralsight.sessionError}<ErrorNote error={pluralsight.sessionError} platform="pluralsight" />{/if}
    <CourseList
      session={pluralsight}
      platform="pluralsight"
      items={courses}
      heading={t("ps_courses")}
      emptyText={t("ps_history_hint")}
    />
    {#if pluralsight.items.length === 0}
      <button type="button" class="btn link" onclick={openHistory}>{t("ps_open_history")}</button>
    {/if}
  {:else}
    <ConnectPanel
      session={pluralsight}
      platformName="Pluralsight"
      title={t("ps_connect_title")}
      body={t("ps_connect_body")}
      loginLabel={t("ps_login_webview")}
      manualLabel={t("ps_cookie_toggle")}
      errorPlatform="pluralsight"
      {manual}
      onWebviewLogin={loginWebview}
    />
  {/if}
</div>

<style>
  .flow {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: var(--s-8);
  }
  .muted,
  .hint {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .steps {
    display: grid;
    gap: var(--s-2);
  }
  .steps li {
    display: grid;
    grid-template-columns: 24px 1fr;
    gap: var(--s-2);
    color: var(--text-muted);
  }
  .steps .n {
    color: var(--text);
  }
  .link {
    justify-self: start;
    color: var(--text-muted);
  }
  .link:hover {
    color: var(--text);
  }
  @media (max-height: 640px) {
    .flow {
      gap: var(--s-5);
    }
  }
</style>
