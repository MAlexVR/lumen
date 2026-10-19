<script lang="ts">
  // The original logo artwork on a 16:10 stage matching its own edge colour.
  import type { Platform } from "../../lib/platforms";

  let { platform, alt = "" }: { platform: Platform; alt?: string } = $props();
</script>

<span class="stage" style:--stage={platform.stageBg} style:--inset="{platform.inset}%">
  <img src={platform.logo} {alt} draggable="false" />
</span>

<style>
  .stage {
    display: block;
    aspect-ratio: 16 / 10;
    background: var(--stage);
    border-radius: var(--r-2);
    overflow: hidden;
    padding: 0 var(--inset, 0%);
  }
  img {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
</style>
