<script lang="ts">
  // Udemy connection wizard: sign in, choose what to check, watch the
  // read-only diagnostic, read the verdict. Nothing is downloaded.
  import Icon, { type IconName } from "../../lib/design/Icon.svelte";
  import ProgressLine from "../../lib/components/ProgressLine.svelte";
  import { t } from "../../lib/i18n/index.svelte";
  import { summarizeReport } from "../../lib/logic/report";
  import { formatTime } from "../../lib/logic/time";
  import { PROGRESS_STEPS, sortCourses, summaryParts, WIZARD_STEPS } from "../../lib/logic/udemy";
  import { LOGIN_TIMEOUT_MS } from "../../lib/state/downloads.svelte";
  import { udemy } from "../../lib/state/udemy.svelte";
  import type { UdemyEndpoint, UdemyVerdict } from "../../lib/types";
  import ErrorNote from "./ErrorNote.svelte";

  let manualOpen = $state(false);
  let cookie = $state("");
  let copied = $state(false);
  let now = $state(Date.now());
  let heading: HTMLElement | undefined = $state();
  const uid = $props.id();

  $effect(() => {
    void udemy.init();
  });

  const step = $derived(udemy.wizard.step);
  const index = $derived(WIZARD_STEPS.indexOf(step));
  const r = $derived(udemy.report);
  const totals = $derived(
    r
      ? {
          courses: r.courses.length,
          verdicts: r.courses.reduce(
            (v, c) => ({ ...v, [c.verdict]: v[c.verdict] + 1 }),
            { downloadable: 0, partial: 0, none: 0, unknown: 0 } as Record<UdemyVerdict, number>,
          ),
        }
      : null,
  );
  const sentence = $derived(totals ? summaryParts(totals) : null);
  const issues = $derived(
    r
      ? summarizeReport({
          id: "udemy",
          platform: "skool",
          group: "",
          cancelled: r.status === "cancelled",
          totalLessons: 0,
          completedLessons: 0,
          complete: true,
          issues: r.issues,
        })
      : null,
  );
  const waitingLeft = $derived(
    udemy.loginStartedAt ? Math.max(0, LOGIN_TIMEOUT_MS - (now - udemy.loginStartedAt)) / 1000 : 0,
  );

  // Move focus to each new step's heading.
  $effect(() => {
    void step;
    queueMicrotask(() => heading?.focus());
  });

  $effect(() => {
    if (udemy.loggingIn !== "webview") return;
    const timer = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(timer);
  });

  $effect(() => {
    if (udemy.error?.code === "bot_challenge" || udemy.error?.code === "login_timeout") manualOpen = true;
  });

  async function submitCookies(e: SubmitEvent) {
    e.preventDefault();
    const text = cookie.trim();
    if (text && (await udemy.login("cookies", text))) cookie = "";
  }

  async function copyPath() {
    if (!udemy.reportPath) return;
    try {
      await navigator.clipboard.writeText(udemy.reportPath);
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {
      /* the path stays selectable */
    }
  }

  const VERDICT_ICON: Record<UdemyVerdict, IconName> = {
    downloadable: "check",
    partial: "alert",
    none: "close",
    unknown: "alert",
  };
  const KNOWN_STATUS = ["ok", "http_401", "http_403", "http_404", "parse_error", "network", "not_tested"];
  const endpointText = (e: UdemyEndpoint) =>
    KNOWN_STATUS.includes(e.status)
      ? t(`ud_es_${e.status}` as "ud_es_ok")
      : t("ud_es_other", { code: e.status.replace("http_", "") });
  const endpointName = (name: string) =>
    ["subscribedCourses", "curriculumItems", "lectureDetail"].includes(name) ? t(`ud_ep_${name}` as "ud_ep_lectureDetail") : name;
</script>

<div class="flow">
  <p class="note"><Icon name="alert" size={16} />{t("ud_note")}</p>

  <nav class="stepper" aria-label={t("ud_steps")}>
    <ol>
      {#each WIZARD_STEPS as s, i (s)}
        <li data-state={i < index ? "done" : i === index ? "current" : "todo"} aria-current={i === index ? "step" : undefined}>
          <span class="dot num" aria-hidden="true">{#if i < index}<Icon name="check" size={12} />{:else}{i + 1}{/if}</span>
          <span class="label">{t(`ud_step_${s}`)}</span>
        </li>
      {/each}
    </ol>
  </nav>
  <p class="visually-hidden" aria-live="polite">
    {t("ud_step_of", { n: index + 1, total: WIZARD_STEPS.length, name: t(`ud_step_${step}`) })}
  </p>

  {#if !udemy.ready}
    <p class="muted" role="status">{t("loading")}</p>
  {:else if step === "login"}
    <section class="panel" aria-labelledby="ud-h-{uid}">
      <h2 id="ud-h-{uid}" tabindex="-1" bind:this={heading}>{t("ud_login_title")}</h2>
      <p class="body">{t("ud_login_body")}</p>
      {#if udemy.loggingIn === "webview"}
        <div class="waiting" role="status">
          <span class="pulse" aria-hidden="true"></span>
          <div>
            <p class="strong">{t("dl_waiting_title")}</p>
            <p class="muted">{t("dl_waiting_body", { name: "Udemy" })}</p>
            <p class="muted">{t("dl_waiting_left")} <span class="num gold">{formatTime(waitingLeft)}</span></p>
          </div>
        </div>
      {:else}
        <div class="actions">
          <button type="button" class="btn btn-primary" onclick={() => udemy.login("webview")} disabled={!!udemy.loggingIn}>
            <Icon name="external" size={16} />{t("ud_login_webview")}
          </button>
        </div>
      {/if}
      {#if udemy.error && !udemy.loggingIn}<ErrorNote error={udemy.error} />{/if}

      <div class="manual">
        <button type="button" class="disclosure" aria-expanded={manualOpen} aria-controls="ud-manual-{uid}" onclick={() => (manualOpen = !manualOpen)}>
          <Icon name="chevronDown" size={16} />{t("ud_cookies_toggle")}
        </button>
        <form id="ud-manual-{uid}" hidden={!manualOpen} onsubmit={submitCookies}>
          <ol class="steps">
            <li><span class="n num">1</span>{t("ud_cookies_step1")}</li>
            <li><span class="n num">2</span>{t("ud_cookies_step2")}</li>
            <li><span class="n num">3</span>{t("ud_cookies_step3")}</li>
          </ol>
          <label for="ud-cookie-{uid}" class="eyebrow">{t("ud_cookies_label")}</label>
          <textarea id="ud-cookie-{uid}" bind:value={cookie} rows="3" autocomplete="off" spellcheck="false" placeholder="access_token=…; client_id=…" disabled={!!udemy.loggingIn}></textarea>
          <div>
            <button type="submit" class="btn btn-quiet" disabled={!cookie.trim() || !!udemy.loggingIn}>
              {t(udemy.loggingIn === "cookies" ? "dl_connecting" : "dl_connect")}
            </button>
          </div>
        </form>
      </div>
    </section>
  {:else if step === "options"}
    <section class="panel" aria-labelledby="ud-h-{uid}">
      <h2 id="ud-h-{uid}" tabindex="-1" bind:this={heading}>{t("ud_step_options")}</h2>
      <p class="body">{t("ud_options_body")}</p>
      <div class="fields">
        <label class="field">
          <span>{t("ud_opt_samples")}</span>
          <input type="number" min="1" max="20" bind:value={udemy.options.sampleLecturesPerCourse} />
        </label>
        <label class="field">
          <span>{t("ud_opt_max")}</span>
          <select
            value={udemy.options.maxCourses ?? ""}
            onchange={(e) => (udemy.options.maxCourses = e.currentTarget.value ? Number(e.currentTarget.value) : null)}
          >
            <option value="">{t("ud_opt_max_all")}</option>
            {#each [5, 10, 25, 50] as n (n)}<option value={n}>{n}</option>{/each}
          </select>
        </label>
        <label class="check">
          <input type="checkbox" bind:checked={udemy.options.includeTitles} />
          <span>
            {t("ud_opt_titles")}
            <span class="muted small">{t("ud_opt_titles_note")}</span>
          </span>
        </label>
      </div>
      {#if udemy.error}<ErrorNote error={udemy.error} />{/if}
      <div class="actions">
        <button type="button" class="btn btn-primary" onclick={() => udemy.diagnose()}>{t("ud_check")}</button>
        <button type="button" class="btn link" onclick={() => udemy.logout()}>{t("ud_logout")}</button>
      </div>
    </section>
  {:else if step === "running"}
    <section class="panel" aria-labelledby="ud-h-{uid}">
      <h2 id="ud-h-{uid}" tabindex="-1" bind:this={heading}>{t("ud_step_running")}</h2>
      <p class="message" aria-live="polite">{udemy.progress.message || t("loading")}</p>
      <ul class="bars">
        {#each PROGRESS_STEPS as s (s)}
          {@const p = udemy.progress.steps[s]}
          <li data-active={udemy.progress.current === s ? "" : undefined}>
            <span class="bname">{t(`ud_ps_${s}`)}</span>
            <ProgressLine value={p.total ? (p.current / p.total) * 100 : 0} label={t(`ud_ps_${s}`)} />
            <span class="num bcount">{p.total ? `${p.current}/${p.total}` : "–"}</span>
          </li>
        {/each}
      </ul>
      <div class="actions">
        <button type="button" class="btn btn-quiet" onclick={() => udemy.cancel()} disabled={udemy.cancelling}>
          <Icon name="close" size={16} />{t(udemy.cancelling ? "ud_cancelling" : "ud_cancel")}
        </button>
      </div>
    </section>
  {:else if r && sentence}
    <section class="panel" aria-labelledby="ud-h-{uid}">
      <h2 id="ud-h-{uid}" tabindex="-1" bind:this={heading} class="verdict">
        {sentence.courses === 1 ? t("ud_result_courses_one") : t("ud_result_courses_n", { n: sentence.courses })}{sentence.parts.length ? ":" : ""}
        {sentence.parts.map((p) => t(p.key, { n: p.n })).join("\u00a0· ")}
      </h2>
      {#if r.status !== "complete"}<p class="partial">{t(r.status === "cancelled" ? "ud_status_cancelled" : "ud_status_aborted")}</p>{/if}
      {#if udemy.error}<ErrorNote error={udemy.error} />{/if}

      <div class="table" role="table" aria-label={t("ud_step_result")}>
        <div class="tr th" role="row">
          <span role="columnheader">{t("ud_col_course")}</span>
          <span role="columnheader">{t("ud_col_verdict")}</span>
          <span role="columnheader">{t("ud_col_types")}</span>
        </div>
        {#each sortCourses(r.courses) as c (c.id)}
          <div class="tr" role="row" data-verdict={c.verdict}>
            <span role="cell" class="cname">{c.title ?? t("ud_course_id", { id: c.id })}</span>
            <span role="cell" class="cverdict"><Icon name={VERDICT_ICON[c.verdict]} size={14} />{t(`ud_verdict_${c.verdict}`)}</span>
            <span role="cell" class="ctypes num">
              {t("ud_types", { video: c.types.video, article: c.types.article, quiz: c.types.quiz })}
              <span class="muted">· {t("ud_probed", { n: c.probed })}</span>
            </span>
          </div>
        {/each}
      </div>

      <section class="endpoints" aria-labelledby="ud-ep-{uid}">
        <h3 id="ud-ep-{uid}" class="eyebrow">{t("ud_endpoints")}</h3>
        <ul>
          {#each r.endpoints as e (e.name)}
            <li data-ok={e.status === "ok"}>
              <Icon name={e.status === "ok" ? "check" : "alert"} size={14} />
              <span class="ename">{endpointName(e.name)}</span>
              <span class="estatus">{endpointText(e)}</span>
            </li>
          {/each}
        </ul>
      </section>

      {#if issues && (issues.errors.length || issues.warnings.length)}
        <details class="issues">
          <summary>{t("ud_issues")} <span class="num muted">({issues.errorCount + issues.warningCount})</span></summary>
          <ul>
            {#each [...issues.errors, ...issues.warnings] as g (g.kind + g.issues[0].severity)}
              {#each g.issues as i, k (k)}
                <li data-severity={i.severity}>{i.message}</li>
              {/each}
            {/each}
          </ul>
        </details>
      {/if}

      <div class="report">
        <span class="eyebrow">{t("ud_report")}</span>
        {#if udemy.reportPath}
          <p class="muted small">{t("ud_report_where")}</p>
          <code>{udemy.reportPath}</code>
        {:else}
          <p class="muted small">{t("ud_report_where_unknown")}</p>
        {/if}
      </div>
      <div class="actions">
        <button type="button" class="btn btn-quiet" onclick={() => udemy.diagnose()}>{t("ud_recheck")}</button>
        {#if udemy.reportPath}
          <button type="button" class="btn link" onclick={copyPath}>{t(copied ? "ud_copied" : "ud_copy_path")}</button>
        {/if}
        <button type="button" class="btn link" onclick={() => udemy.dispatch({ type: "back" })}>{t("ud_back")}</button>
      </div>
    </section>
  {/if}
</div>

<style>
  .flow {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: var(--s-6);
    max-width: 900px;
  }
  .note {
    display: flex;
    align-items: flex-start;
    gap: var(--s-2);
    font-size: var(--fs-sm);
    color: var(--text-muted);
    padding: var(--s-3) var(--s-4);
    border-left: 2px solid var(--moon);
    background: var(--surface-1);
    border-radius: 0 var(--r-2) var(--r-2) 0;
  }
  .note :global(svg) {
    flex: none;
    margin-top: 2px;
    color: var(--moon);
  }
  .stepper ol {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-2) var(--s-6);
    padding-bottom: var(--s-3);
    border-bottom: 1px solid var(--line);
  }
  .stepper li {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    font-size: var(--fs-sm);
    color: var(--text-muted);
  }
  .dot {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    border: 1px solid var(--line);
    font-size: var(--fs-2xs);
  }
  [data-state="current"] {
    color: var(--text);
  }
  [data-state="current"] .dot {
    border-color: var(--lumen);
    color: var(--lumen);
  }
  [data-state="done"] .dot {
    border-color: var(--ok);
    color: var(--ok);
  }
  .panel {
    display: grid;
    gap: var(--s-5);
  }
  h2 {
    font-size: var(--fs-xl);
  }
  .verdict {
    font-size: var(--fs-lg);
    line-height: var(--lh-snug);
  }
  .body {
    color: var(--text-muted);
    max-width: 60ch;
  }
  .muted {
    color: var(--text-muted);
  }
  .small {
    font-size: var(--fs-xs);
    display: block;
  }
  .strong {
    color: var(--text);
  }
  .gold {
    color: var(--lumen);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-2);
    align-items: center;
  }
  .link {
    color: var(--text-muted);
    padding: 0 var(--s-3);
  }
  .link:hover {
    color: var(--text);
  }
  .waiting {
    display: flex;
    gap: var(--s-4);
    padding: var(--s-4) var(--s-5);
    border: 1px solid var(--line);
    border-radius: var(--r-3);
    background: var(--surface-1);
    justify-self: start;
  }
  .pulse {
    flex: none;
    width: 10px;
    height: 10px;
    margin-top: 6px;
    border-radius: 50%;
    background: var(--lumen);
  }
  .manual {
    border-top: 1px solid var(--line);
    padding-top: var(--s-3);
  }
  .disclosure {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    height: 36px;
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .disclosure[aria-expanded="true"] :global(svg) {
    transform: rotate(180deg);
  }
  form {
    display: grid;
    gap: var(--s-3);
    padding-top: var(--s-3);
  }
  form[hidden] {
    display: none;
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
    font-size: var(--fs-sm);
  }
  .n {
    color: var(--text);
    font-weight: var(--fw-numeral);
  }
  label.eyebrow {
    font-family: var(--font-ui);
  }
  textarea,
  input[type="number"],
  select {
    background: var(--surface-1);
    color: var(--text);
    border: 1px solid var(--line);
    border-radius: var(--r-2);
    font: var(--fs-sm) / 1.5 var(--font-ui);
    padding: var(--s-2) var(--s-3);
  }
  textarea {
    width: 100%;
    min-height: 76px;
    resize: vertical;
  }
  textarea:focus-visible,
  input:focus-visible,
  select:focus-visible {
    border-color: var(--lumen);
    outline: none;
  }
  .fields {
    display: grid;
    gap: var(--s-4);
  }
  .field {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-3);
    font-size: var(--fs-sm);
  }
  .field input {
    width: 80px;
  }
  .check {
    display: flex;
    gap: var(--s-2);
    font-size: var(--fs-sm);
    max-width: 60ch;
  }
  .check input {
    margin-top: 3px;
    accent-color: var(--lumen);
  }
  .message {
    color: var(--text);
    min-height: 1.5em;
  }
  .bars {
    display: grid;
    gap: var(--s-3);
  }
  .bars li {
    display: grid;
    grid-template-columns: 150px minmax(0, 1fr) 70px;
    align-items: center;
    gap: var(--s-4);
    font-size: var(--fs-sm);
    color: var(--text-muted);
  }
  .bars li[data-active] {
    color: var(--text);
  }
  .bcount {
    text-align: right;
    font-size: var(--fs-xs);
  }
  .partial {
    color: var(--moon);
    font-size: var(--fs-sm);
  }
  .table {
    display: grid;
    border-top: 1px solid var(--line);
  }
  .tr {
    display: grid;
    grid-template-columns: minmax(0, 1.2fr) minmax(110px, 0.6fr) minmax(0, 1.4fr);
    gap: var(--s-4);
    padding: var(--s-2) 0;
    border-bottom: 1px solid var(--line-soft);
    font-size: var(--fs-sm);
    align-items: center;
  }
  .th {
    font-size: var(--fs-2xs);
    letter-spacing: var(--tracking-label);
    text-transform: uppercase;
    color: var(--text-muted);
  }
  .cname {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cverdict {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
  }
  [data-verdict="downloadable"] .cverdict {
    color: var(--ok);
  }
  [data-verdict="partial"] .cverdict {
    color: var(--moon);
  }
  [data-verdict="none"] .cverdict {
    color: var(--danger);
  }
  [data-verdict="unknown"] .cverdict {
    color: var(--text-muted);
  }
  .ctypes {
    font-size: var(--fs-xs);
  }
  .endpoints {
    display: grid;
    gap: var(--s-2);
  }
  .endpoints h3 {
    font-family: var(--font-ui);
  }
  .endpoints li {
    display: grid;
    grid-template-columns: 18px minmax(0, 1fr) auto;
    gap: var(--s-2);
    align-items: center;
    font-size: var(--fs-sm);
    padding: var(--s-1) 0;
  }
  .endpoints [data-ok="true"] :global(svg) {
    color: var(--ok);
  }
  .endpoints [data-ok="false"] {
    color: var(--danger);
  }
  .estatus {
    font-size: var(--fs-xs);
  }
  .issues summary {
    cursor: pointer;
    font-size: var(--fs-sm);
  }
  .issues ul {
    display: grid;
    gap: var(--s-1);
    padding: var(--s-2) 0 0 var(--s-4);
    font-size: var(--fs-xs);
    color: var(--text-muted);
  }
  .issues [data-severity="error"] {
    color: var(--text);
  }
  .report {
    display: grid;
    gap: var(--s-1);
  }
  .report .eyebrow {
    font-family: var(--font-ui);
  }
  code {
    font-family: ui-monospace, "SFMono-Regular", "Cascadia Mono", "DejaVu Sans Mono", monospace;
    font-size: var(--fs-xs);
    overflow-wrap: anywhere;
    user-select: all;
  }
  @media (max-width: 760px) {
    .tr {
      grid-template-columns: minmax(0, 1fr) auto;
    }
    .ctypes,
    .th span:last-child {
      grid-column: 1 / -1;
    }
    .th span:last-child {
      display: none;
    }
    .bars li {
      grid-template-columns: 110px minmax(0, 1fr) 56px;
    }
  }
</style>
