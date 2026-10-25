<script lang="ts">
  // Golden Life flow: email + password sign-in; the password lives only in this component state.
  import BrandMark from "../../lib/components/BrandMark.svelte";
  import { t } from "../../lib/i18n/index.svelte";
  import { canSubmitCredentials, displayGoldenlifeCourses } from "../../lib/logic/goldenlife";
  import { app } from "../../lib/state/app.svelte";
  import { goldenlife } from "../../lib/state/downloads.svelte";
  import CourseList from "./CourseList.svelte";
  import type { CourseListItem } from "./CourseList.svelte";
  import ErrorNote from "./ErrorNote.svelte";
  import SessionBar from "./SessionBar.svelte";

  let email = $state("");
  let password = $state("");
  const uid = $props.id();
  const courses = $derived<CourseListItem[]>(displayGoldenlifeCourses(goldenlife.items, t("gl_course_missing_id")));
  const busy = $derived(goldenlife.session === "connecting");

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    const api = app.api;
    const secret = password;
    password = "";
    if (!api || !canSubmitCredentials(email, secret)) return;
    await goldenlife.loginManual(async () => (await api.goldenlifeLoginCredentials(email.trim(), secret)).courses);
  }

  async function openAcademy() {
    await app.api?.openUrl("https://my.goldenlife-academy.com/").catch(() => {});
  }
</script>

<div class="flow">
  {#if goldenlife.session === "unknown"}
    <p class="muted" role="status">{t("loading")}</p>
  {:else if goldenlife.session === "connected"}
    <SessionBar session={goldenlife} label={t("gl_connected")} confirmLabel={t("gl_logout_confirm")} />
    {#if goldenlife.sessionError}<ErrorNote error={goldenlife.sessionError} platform="goldenlife" />{/if}
    <CourseList
      session={goldenlife}
      platform="goldenlife"
      items={courses}
      heading={t("gl_courses")}
      emptyText={t("gl_no_courses")}
    />
    {#if goldenlife.items.length === 0}
      <button type="button" class="btn link" onclick={openAcademy}>{t("gl_open_academy")}</button>
    {/if}
  {:else}
    <section class="connect" aria-labelledby="gl-connect-{uid}">
      <span class="halo" aria-hidden="true"></span>
      <div class="lead">
        <BrandMark size={36} />
        <h2 id="gl-connect-{uid}">{t("gl_connect_title")}</h2>
        <p class="body">{t("gl_connect_body")}</p>
        <form onsubmit={submit}>
          <label for="gl-email-{uid}" class="eyebrow">{t("gl_email_label")}</label>
          <input
            id="gl-email-{uid}"
            type="email"
            bind:value={email}
            autocomplete="username"
            autocapitalize="off"
            spellcheck="false"
            placeholder={t("gl_email_placeholder")}
            disabled={busy}
          />
          <label for="gl-password-{uid}" class="eyebrow">{t("gl_password_label")}</label>
          <input
            id="gl-password-{uid}"
            type="password"
            bind:value={password}
            autocomplete="current-password"
            spellcheck="false"
            disabled={busy}
          />
          <p class="hint">{t("gl_privacy")}</p>
          <div>
            <button type="submit" class="btn btn-primary" disabled={!canSubmitCredentials(email, password) || busy}>
              {t(busy ? "dl_connecting" : "gl_sign_in")}
            </button>
          </div>
        </form>
        {#if goldenlife.sessionError && !busy}
          <ErrorNote error={goldenlife.sessionError} platform="goldenlife" />
        {/if}
      </div>
    </section>
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
  .connect {
    position: relative;
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
    gap: var(--s-4);
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
  form {
    display: grid;
    gap: var(--s-3);
    width: min(100%, 420px);
  }
  label {
    font-family: var(--font-ui);
  }
  input {
    width: 100%;
    padding: var(--s-3);
    background: var(--surface-1);
    color: var(--text);
    border: 1px solid var(--line);
    border-radius: var(--r-2);
    font: var(--fs-sm) / 1.5 var(--font-ui);
  }
  input:focus-visible {
    border-color: var(--lumen);
    outline: none;
  }
  input::placeholder {
    color: var(--text-muted);
  }
  .lead :global(.note) {
    width: min(100%, 520px);
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
