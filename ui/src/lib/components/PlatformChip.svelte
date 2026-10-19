<script lang="ts">
  // The one platform brand chip: same box for every platform, wordmark fitted
  // inside (per-platform optical scale lives in the registry).
  import { platformById } from "../platforms";

  let { id }: { id: string } = $props();
  const mark = $derived(platformById(id)?.mark);
</script>

{#if mark}
  <span class="chip" style:--stage={mark.bg} style:--scale={mark.scale}><img src={mark.src} alt="" /></span>
{/if}

<style>
  .chip {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: none;
    box-sizing: border-box;
    width: 60px;
    height: 24px;
    padding: 4px;
    border-radius: var(--r-1);
    overflow: hidden;
    background: var(--stage);
    outline: 1px solid var(--line-soft);
  }
  img {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: contain;
    transform: scale(var(--scale));
  }
</style>
