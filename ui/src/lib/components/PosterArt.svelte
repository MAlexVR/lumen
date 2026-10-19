<script lang="ts">
  // Generated poster: one of six tone-on-tone motifs chosen from the title
  // hash, a faint accent glow, the course number and small initials.
  // Strictly palette tokens; replaced by real video stills in S3.
  import { initials, posterSeed } from "../logic/poster";

  interface Props {
    title: string;
    index: number;
    /** "poster" is 2:3, "still" is 16:9. */
    shape?: "poster" | "still";
    /** Stronger light (continue hero). */
    glow?: boolean;
  }
  let { title, index, shape = "poster", glow = false }: Props = $props();
  const uid = $props.id();

  const W = $derived(shape === "poster" ? 200 : 320);
  const H = $derived(shape === "poster" ? 300 : 180);
  const seed = $derived(posterSeed(title));
  const letters = $derived(initials(title));
  const cx = $derived(seed.x * W);
  const cy = $derived(seed.y * H);
  const s = $derived(seed.scale);
  const range = (n: number) => Array.from({ length: n }, (_, i) => i);
</script>

<svg viewBox="0 0 {W} {H}" preserveAspectRatio="xMidYMid slice" aria-hidden="true" focusable="false">
  <defs>
    <radialGradient id="g-{uid}" cx={seed.x} cy={seed.y} r="0.75">
      <stop offset="0" stop-color="var(--{seed.glow === 'moon' ? 'moon' : 'lumen'})" stop-opacity={glow ? 0.16 : 0.07} />
      <stop offset="1" stop-color="var(--{seed.glow === 'moon' ? 'moon' : 'lumen'})" stop-opacity="0" />
    </radialGradient>
  </defs>
  <rect width={W} height={H} fill="var(--surface-2)" />
  <rect width={W} height={H} fill="url(#g-{uid})" />

  <g fill="none" stroke="var(--line)" stroke-width="1">
    {#if seed.motif === "arcs"}
      {#each range(7) as i (i)}
        {@const r = (i + 1) * 0.15 * H * s}
        <path d="M {cx - r} {H} A {r} {r} 0 0 1 {cx + r} {H}" stroke={i === 2 ? "var(--ink-line)" : "var(--line)"} />
      {/each}
    {:else if seed.motif === "rays"}
      {#each range(15) as i (i)}
        {@const a = Math.PI * (1.08 + (i / 14) * 0.84)}
        <line x1={cx} y1={H} x2={cx + Math.cos(a) * H * 1.4} y2={H + Math.sin(a) * H * 1.4} />
      {/each}
      <circle cx={cx} cy={H} r={H * 0.14 * s} fill="var(--surface-3)" stroke="var(--ink-line)" />
    {:else if seed.motif === "dots"}
      {@const step = 14 * s}
      {#each range(Math.ceil(W / step) + 1) as ix (ix)}
        {#each range(Math.ceil(H / step) + 1) as iy (iy)}
          {@const px = ix * step + 6}
          {@const py = iy * step + 6}
          {@const near = Math.hypot(px - cx, py - cy) < H * 0.28 * s}
          <circle cx={px} cy={py} r={near ? 1.7 : 1} fill={near ? "var(--ink-line)" : "var(--line)"} stroke="none" />
        {/each}
      {/each}
    {:else if seed.motif === "diagonals"}
      {#each range(13) as i (i)}
        {@const off = cx - H * 0.4 + (i - 6) * 7 * s}
        <line x1={off - H * 0.35} y1={H} x2={off + H * 0.75} y2="0" stroke={i === 6 ? "var(--ink-line)" : "var(--line)"} />
      {/each}
      {#each range(5) as i (i)}
        {@const off = cx + H * 0.15 + i * 22 * s}
        <line x1={off - H * 0.35} y1={H} x2={off + H * 0.75} y2="0" stroke="var(--line)" />
      {/each}
    {:else if seed.motif === "horizon"}
      {@const hy = H * (0.45 + seed.y * 0.3)}
      <circle cx={cx} cy={hy} r={H * 0.16 * s} fill="var(--surface-3)" stroke="none" />
      <rect x="0" y={hy} width={W} height={H - hy} fill="var(--surface-2)" />
      {#each range(9) as i (i)}
        {@const y = hy + Math.pow(i / 8, 1.7) * (H - hy)}
        <line x1="0" x2={W} y1={y} y2={y} stroke={i === 0 ? "var(--ink-line)" : "var(--line)"} />
      {/each}
    {:else}
      {#each range(4) as i (i)}
        <ellipse
          cx={cx}
          cy={cy}
          rx={H * (0.16 + i * 0.12) * s}
          ry={H * (0.06 + i * 0.045) * s}
          transform="rotate(-18 {cx} {cy})"
          stroke={i === 1 ? "var(--ink-line)" : "var(--line)"}
        />
      {/each}
      <circle cx={cx} cy={cy} r={H * 0.05 * s} fill="var(--surface-3)" stroke="var(--ink-line)" />
    {/if}
  </g>

  <text x="14" y="26" class="idx">{String(index).padStart(2, "0")}</text>
  {#if shape === "poster"}
    <text x="14" y={H - 16} class="letters" font-size="36">{letters}</text>
  {:else}
    <text x={W - 14} y="38" text-anchor="end" class="letters" font-size="28">{letters}</text>
  {/if}
</svg>

<style>
  svg {
    display: block;
    width: 100%;
    height: 100%;
  }
  .idx {
    font-family: var(--font-ui);
    font-size: 11px;
    font-weight: 500;
    letter-spacing: 0.12em;
    fill: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .letters {
    font-family: var(--font-display);
    fill: var(--ink-strong);
    font-weight: var(--fw-display);
    letter-spacing: -0.02em;
  }
</style>
