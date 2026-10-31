<script lang="ts">
  // Quiet corner readout over the video: the workout timer and the set the
  // viewer is on, with a button to mark it done and move to the next one.
  import Icon from "../../lib/design/Icon.svelte";
  import { t } from "../../lib/i18n/index.svelte";
  import { targetLabel } from "../../lib/logic/exercisePlan";
  import { elapsedMs, formatElapsed, type SetProgress, type WorkoutSession } from "../../lib/logic/workoutSession";
  import type { ExercisePlan } from "../../lib/types";

  interface Props {
    session: WorkoutSession;
    plan: ExercisePlan | null;
    progress: SetProgress | null;
    /** The viewer has not moved for a while: the readout fades back. */
    idle: boolean;
    hasNext: boolean;
    ontimer: () => void;
    onreset: () => void;
    onset: () => void;
    onundo: () => void;
    onnext: () => void;
  }
  let { session, plan, progress, idle, hasNext, ontimer, onreset, onset, onundo, onnext }: Props = $props();

  let now = $state(Date.now());
  const running = $derived(session.runningSince !== null);
  const paused = $derived(session.started && !running);
  // Only a running timer needs to tick.
  $effect(() => {
    if (!running) return;
    now = Date.now();
    const id = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(id);
  });
  const elapsed = $derived(formatElapsed(elapsedMs(session, running ? now : (session.runningSince ?? now))));
  const reps = $derived(plan ? targetLabel(plan.target, t("hud_reps")) : "");
</script>

<div class="hud" class:idle role="group" aria-label={t("hud_label")}>
  <div class="chip timer">
    <button
      type="button"
      class="plain"
      aria-pressed={paused}
      aria-label={t(paused ? "hud_timer_resume" : "hud_timer_pause")}
      title={t(paused ? "hud_timer_resume" : "hud_timer_pause")}
      onclick={ontimer}
    >
      <span class="dot" class:on={running} aria-hidden="true"></span>
      <span class="num" aria-live="off">{elapsed}</span>
    </button>
    {#if paused}
      <button type="button" class="plain small" aria-label={t("hud_reset")} title={t("hud_reset")} onclick={onreset}>
        <Icon name="close" size={14} />
      </button>
    {/if}
  </div>

  {#if plan && progress}
    <div class="chip sets" aria-live="polite">
      {#if progress.finished}
        <span class="ok"><Icon name="check" size={14} />{t("hud_finished")}</span>
        {#if hasNext}
          <button type="button" class="pill" onclick={onnext}>{t("hud_next")}</button>
        {/if}
      {:else}
        <span class="label">{t("hud_set")}</span>
        <b class="num now">{progress.current}{progress.total ? `/${progress.total}` : ""}</b>
        {#if reps}<span class="dotsep" aria-hidden="true">·</span><span class="reps">{reps}</span>{/if}
        <button type="button" class="done" aria-label={t("hud_set_done")} title="{t('hud_set_done')} (S)" onclick={onset}>
          <Icon name="check" size={16} />
        </button>
      {/if}
      {#if progress.done > 0}
        <button type="button" class="plain small" aria-label={t("hud_undo")} title={t("hud_undo")} onclick={onundo}>
          <Icon name="back10" size={14} />
        </button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .hud {
    position: absolute;
    top: var(--s-4);
    right: var(--s-4);
    z-index: 4;
    display: grid;
    justify-items: end;
    gap: var(--s-2);
    transition: opacity var(--dur-3) var(--ease-out);
  }
  .hud.idle {
    opacity: 0.55;
  }
  .hud:hover,
  .hud:focus-within {
    opacity: 1;
  }
  .chip {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-height: 32px;
    padding: 0 var(--s-3);
    border: 1px solid var(--line);
    border-radius: 999px;
    background: color-mix(in srgb, var(--scrim) 85%, transparent);
    backdrop-filter: blur(8px);
    color: var(--text);
    font-size: var(--fs-sm);
  }
  .plain {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    color: inherit;
  }
  .plain.small {
    color: var(--text-muted);
  }
  .plain:hover,
  .plain:focus-visible {
    color: var(--text);
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--text-faint);
  }
  .dot.on {
    background: var(--lumen);
  }
  .label,
  .reps {
    color: var(--text-muted);
  }
  .now {
    color: var(--lumen);
    font-weight: 600;
  }
  .dotsep {
    color: var(--text-faint);
  }
  .done {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    margin-right: calc(var(--s-1) * -1);
    border-radius: 50%;
    border: 1px solid var(--lumen);
    color: var(--lumen);
  }
  .done:hover,
  .done:focus-visible {
    background: var(--lumen);
    color: var(--bg);
  }
  .ok {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    color: var(--ok);
  }
  .pill {
    padding: 2px var(--s-3);
    border: 1px solid var(--line);
    border-radius: 999px;
    color: var(--text);
  }
  .pill:hover,
  .pill:focus-visible {
    border-color: var(--lumen);
    color: var(--lumen);
  }
</style>
