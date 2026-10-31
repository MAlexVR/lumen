<script lang="ts">
  // Harbiz flow: email + password sign-in; the password lives only in this component state.
  import { t } from "../../lib/i18n/index.svelte";
  import { canSubmitCredentials } from "../../lib/logic/goldenlife";
  import { displayHarbizCourses } from "../../lib/logic/harbiz";
  import harbizLogo from "../../assets/platforms/harbiz.webp";
  import { app } from "../../lib/state/app.svelte";
  import { harbiz } from "../../lib/state/downloads.svelte";
  import CourseList from "./CourseList.svelte";
  import type { CourseListItem } from "./CourseList.svelte";
  import ErrorNote from "./ErrorNote.svelte";
  import SessionBar from "./SessionBar.svelte";

  let email = $state("");
  let password = $state("");
  const uid = $props.id();
  const courses = $derived<CourseListItem[]>(
    displayHarbizCourses(harbiz.items, {
      workouts: (n) => t("hb_workouts", { n: String(n) }),
      pending: (n) => t("hb_pending", { n: String(n) }),
      upToDate: t("hb_up_to_date"),
      range: (first, last) => t("hb_range", { first, last }),
      empty: t("hb_empty"),
    }).map((course) => ({ ...course, coverUrl: harbizLogo })),
  );
  const busy = $derived(harbiz.session === "connecting");

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    const api = app.api;
    const secret = password;
    password = "";
    if (!api || !canSubmitCredentials(email, secret)) return;
    await harbiz.loginManual(async () => (await api.harbizLoginCredentials(email.trim(), secret)).courses);
  }

  async function openApp() {
    await app.api?.openUrl("https://app.harbiz.io/").catch(() => {});
  }
</script>

<div class="flow">
  {#if harbiz.session === "unknown"}
    <p class="muted" role="status">{t("loading")}</p>
  {:else if harbiz.session === "connected"}
    <SessionBar session={harbiz} label={t("hb_connected")} confirmLabel={t("hb_logout_confirm")} />
    {#if harbiz.sessionError}<ErrorNote error={harbiz.sessionError} platform="harbiz" />{/if}
    <CourseList
      session={harbiz}
      platform="harbiz"
      items={courses}
      heading={t("hb_courses")}
      emptyText={t("hb_no_courses")}
    />
    {#if harbiz.items.length === 0}
      <button type="button" class="btn link" onclick={openApp}>{t("hb_open_app")}</button>
    {/if}
  {:else}
    <section class="connect" aria-labelledby="hb-connect-{uid}">
      <span class="halo" aria-hidden="true"></span>
      <div class="lead">
        <h2 id="hb-connect-{uid}">{t("hb_connect_title")}</h2>
        <p class="body">{t("hb_connect_body")}</p>
        <form onsubmit={submit}>
          <label for="hb-email-{uid}" class="eyebrow">{t("hb_email_label")}</label>
          <input
            id="hb-email-{uid}"
            type="email"
            bind:value={email}
            autocomplete="username"
            autocapitalize="off"
            spellcheck="false"
            placeholder={t("hb_email_placeholder")}
            disabled={busy}
          />
          <label for="hb-password-{uid}" class="eyebrow">{t("hb_password_label")}</label>
          <input
            id="hb-password-{uid}"
            type="password"
            bind:value={password}
            autocomplete="current-password"
            spellcheck="false"
            disabled={busy}
          />
          <p class="hint">{t("hb_privacy")}</p>
          <div>
            <button type="submit" class="btn btn-primary" disabled={!canSubmitCredentials(email, password) || busy}>
              {t(busy ? "dl_connecting" : "hb_sign_in")}
            </button>
          </div>
        </form>
        {#if harbiz.sessionError && !busy}
          <ErrorNote error={harbiz.sessionError} platform="harbiz" />
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
