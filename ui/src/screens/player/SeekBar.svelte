<script lang="ts">
  // Seek bar: native range input (keyboard + a11y) over a hairline track
  // showing buffered and played ranges, with a hover time preview.
  import { t } from "../../lib/i18n/index.svelte";
  import { formatTime } from "../../lib/logic/time";

  interface Props {
    current: number;
    duration: number;
    buffered: number;
    onseek: (seconds: number) => void;
  }
  let { current, duration, buffered, onseek }: Props = $props();
  let hover = $state<{ x: number; time: number } | null>(null);
  let width = $state(1);

  const d = $derived(Number.isFinite(duration) && duration > 0 ? duration : 0);
  const played = $derived(d ? (current / d) * 100 : 0);
  const loaded = $derived(d ? Math.min(100, (buffered / d) * 100) : 0);

  function move(e: PointerEvent) {
    if (!d) return;
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const x = Math.max(0, Math.min(rect.width, e.clientX - rect.left));
    hover = { x, time: (x / rect.width) * d };
  }
</script>

<div
  class="seek"
  bind:clientWidth={width}
  onpointermove={move}
  onpointerleave={() => (hover = null)}
  role="presentation"
>
  <div class="track" aria-hidden="true">
    <div class="buffered" style:width="{loaded}%"></div>
    <div class="played" style:width="{played}%"></div>
  </div>
  <input
    type="range"
    min="0"
    max={d}
    step="any"
    value={current}
    disabled={!d}
    aria-label={t("seek")}
    aria-valuetext={t("time_of", { a: formatTime(current), b: formatTime(d) })}
    oninput={(e) => onseek(Number(e.currentTarget.value))}
  />
  {#if hover}
    <span class="tip num" style:left="{Math.max(24, Math.min(width - 24, hover.x))}px">{formatTime(hover.time)}</span>
  {/if}
</div>

<style>
  .seek {
    position: relative;
    height: 18px;
    display: flex;
    align-items: center;
  }
  .track {
    position: absolute;
    left: 0;
    right: 0;
    height: 2px;
    background: var(--line);
    transition: height var(--dur-1) var(--ease-out);
  }
  .seek:hover .track {
    height: 4px;
  }
  .buffered,
  .played {
    position: absolute;
    inset: 0 auto 0 0;
  }
  .buffered {
    background: var(--text-faint);
  }
  .played {
    background: var(--lumen);
  }
  input {
    position: relative;
    width: 100%;
    height: 18px;
    margin: 0;
    background: transparent;
    -webkit-appearance: none;
    appearance: none;
    cursor: pointer;
  }
  input::-webkit-slider-runnable-track {
    background: transparent;
    height: 18px;
  }
  input::-webkit-slider-thumb {
    -webkit-appearance: none;
    width: 12px;
    height: 12px;
    margin-top: 3px;
    border-radius: 50%;
    background: var(--lumen);
    opacity: 0;
    transition: opacity var(--dur-1) var(--ease-out);
  }
  input::-moz-range-thumb {
    width: 12px;
    height: 12px;
    border: 0;
    border-radius: 50%;
    background: var(--lumen);
  }
  .seek:hover input::-webkit-slider-thumb,
  input:focus-visible::-webkit-slider-thumb {
    opacity: 1;
  }
  .tip {
    position: absolute;
    bottom: 22px;
    transform: translateX(-50%);
    font-size: var(--fs-xs);
    background: var(--surface-3);
    color: var(--text);
    padding: 2px var(--s-2);
    border-radius: var(--r-1);
    pointer-events: none;
  }
</style>
