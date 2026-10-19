<script lang="ts">
  // Hairline progress. Always paired with a visible text state by callers.
  interface Props {
    value: number;
    label: string;
    thickness?: number;
  }
  let { value, label, thickness = 2 }: Props = $props();
  const pct = $derived(Math.max(0, Math.min(100, Math.round(value))));
</script>

<div
  class="line"
  role="progressbar"
  aria-label={label}
  aria-valuemin={0}
  aria-valuemax={100}
  aria-valuenow={pct}
  style:--h="{thickness}px"
>
  <div class="fill" style:width="{pct}%"></div>
</div>

<style>
  .line {
    position: relative;
    height: var(--h);
    background: var(--line);
    overflow: hidden;
  }
  .fill {
    position: absolute;
    inset: 0 auto 0 0;
    background: var(--lumen);
    transition: width var(--dur-3) var(--ease-out);
  }
</style>
