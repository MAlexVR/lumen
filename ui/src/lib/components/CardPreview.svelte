<script lang="ts" module>
  // Only one card plays a hover preview at a time, across the whole app.
  const player = $state<{ id: string | null }>({ id: null });
</script>

<script lang="ts">
  // Course card art over the generated poster: a cached thumbnail (one image,
  // requested when the card nears the viewport) and, after a short dwell on
  // hover/focus, a muted 5 s preview. The <video> exists only while previewing.
  import { onDestroy } from "svelte";
  import { initialHover, reduceHover, type HoverAction, type HoverState } from "../logic/cardPreview";
  import { previewTime } from "../logic/preview";
  import { thumbAt, thumbs } from "../state/thumbs.svelte";

  interface Props {
    /** Media server URL of the lesson video (null: still image only). */
    src: string | null;
    /** Library-relative path of the same video (thumbnail key). */
    path: string | null;
    /** The video's own thumbnail file (media server URL): used before asking for a generated one. */
    still?: string | null;
    /** Saved resume position of that lesson (seconds). */
    resumeAt: number;
    /** Card hovered or focused (set by the parent card). */
    hot: boolean;
    /** Card number, redrawn above the image (the poster's own is covered). */
    index: number;
    /** Thumbnail width to request (bigger posters ask for more). */
    width?: number;
  }
  let { src, path, still = null, resumeAt, hot, index, width = 480 }: Props = $props();

  const id = $props.id();
  const WINDOW = 5;
  const DWELL_MS = 300;
  let host: HTMLElement | undefined = $state();
  let video: HTMLVideoElement | undefined = $state();
  let near = $state(false);
  let loadedUrl = $state<string | null>(null);
  let playing = $state(false);
  let hover = $state<HoverState>(initialHover);
  let start = 0;
  let dwell: ReturnType<typeof setTimeout> | undefined;

  let stillFailed = $state(false);
  const useStill = $derived(!!still && !stillFailed);
  const item = $derived(path ? { path, atSecs: thumbAt(resumeAt), width } : null);
  const thumb = $derived(
    useStill && still
      ? { state: "ready" as const, url: still }
      : near && item
        ? thumbs.lookup(item)
        : { state: "idle" as const },
  );
  const imgLoaded = $derived(thumb.state === "ready" && loadedUrl === thumb.url);
  const shown = $derived(imgLoaded || playing);
  const hoverDispatch = (a: HoverAction) => (hover = reduceHover(hover, a));

  // Near the viewport: ask the broker for this card's thumbnail; leave on exit.
  $effect(() => {
    if (!host) return;
    const io = new IntersectionObserver((entries) => (near = entries.some((e) => e.isIntersecting)), {
      rootMargin: "300px 0px",
    });
    io.observe(host);
    return () => io.disconnect();
  });

  $effect(() => {
    const it = item;
    if (!near || !it || useStill) return;
    void thumbs.generation;
    thumbs.want(it);
    return () => thumbs.leave(it);
  });

  // Hover/focus: after a short dwell, one card at a time, never on touch or reduced motion.
  $effect(() => {
    clearTimeout(dwell);
    if (hot && near && src) {
      dwell = setTimeout(() => {
        hoverDispatch({
          type: "enter",
          canHover: matchMedia("(hover: hover)").matches,
          reducedMotion: matchMedia("(prefers-reduced-motion: reduce)").matches,
        });
        if (hover.mode === "playing") player.id = id;
      }, DWELL_MS);
    } else {
      hoverDispatch({ type: "leave" });
      if (player.id === id) player.id = null;
    }
  });

  const previewing = $derived(hover.mode === "playing" && player.id === id && near && !!src);

  $effect(() => {
    if (!previewing) playing = false;
  });

  function onMeta() {
    if (!video) return;
    start = previewTime(resumeAt, video.duration);
    if (start > 0) video.currentTime = start;
    void video.play().catch(() => {});
  }

  function onTime() {
    if (!video) return;
    if (video.currentTime > start) playing = true;
    hoverDispatch({ type: "tick", elapsed: video.currentTime - start, window: WINDOW });
    if (hover.mode === "playing" && hover.restart) {
      video.currentTime = start;
      hover = { mode: "playing" };
    }
  }

  onDestroy(() => {
    clearTimeout(dwell);
    if (player.id === id) player.id = null;
  });
</script>

<span class="art-layer" bind:this={host} data-shown={shown}>
  {#if thumb.state === "ready"}
    {@const url = thumb.url}
    <img
      src={url}
      alt=""
      decoding="async"
      loading="lazy"
      draggable="false"
      class:on={imgLoaded}
      onload={() => (loadedUrl = url)}
      onerror={() => (useStill ? (stillFailed = true) : item && thumbs.fail(item))}
    />
  {/if}
  {#if previewing}
    <video
      bind:this={video}
      class:on={playing}
      {src}
      muted
      playsinline
      preload="metadata"
      disablepictureinpicture
      disableremoteplayback
      tabindex="-1"
      aria-hidden="true"
      onloadedmetadata={onMeta}
      ontimeupdate={onTime}
      onerror={() => (playing = false)}
    ></video>
  {/if}
  {#if shown}<span class="idx num">{String(index).padStart(2, "0")}</span>{/if}
</span>

<style>
  .art-layer {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }
  img,
  video {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    opacity: 0;
    filter: brightness(0.78);
    transition: opacity var(--dur-3) var(--ease-out);
  }
  video {
    background: var(--bg);
  }
  img.on,
  video.on {
    opacity: 1;
  }
  /* Keep the card number on top and the progress line at the bottom legible. */
  .art-layer[data-shown="true"]::before,
  .art-layer[data-shown="true"]::after {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    z-index: 1;
  }
  .art-layer[data-shown="true"]::before {
    top: 0;
    height: 28%;
    background: linear-gradient(to bottom, var(--scrim), transparent);
  }
  .art-layer[data-shown="true"]::after {
    bottom: 0;
    height: 30%;
    background: linear-gradient(to top, var(--scrim), transparent);
  }
  .idx {
    position: absolute;
    top: 8px;
    left: 9px;
    z-index: 2;
    font-size: 10px;
    letter-spacing: 0.12em;
    color: var(--text);
  }
</style>
