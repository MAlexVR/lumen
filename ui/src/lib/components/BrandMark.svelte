<script lang="ts">
  // The Lumen badge for small placements: the owner's logo without the four
  // platform tiles (assets/lumen-logo-small.svg), which turn into noise
  // below ~48px. The full logo is the app icon.
  import mark from "../../assets/brand/lumen-mark.svg";

  let { size = 28 }: { size?: number } = $props();
</script>

<img src={mark} width={size} height={size} alt="" aria-hidden="true" draggable="false" />

<style>
  img {
    display: block;
    flex: none;
  }
</style>
