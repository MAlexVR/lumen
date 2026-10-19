<script lang="ts">
  // Video on black with custom controls, keyboard map and resume handling.
  import { onDestroy } from "svelte";
  import Icon from "../../lib/design/Icon.svelte";
  import { t } from "../../lib/i18n/index.svelte";
  import { actionForKey, nextSpeed, SPEEDS, type PlayerAction } from "../../lib/logic/keymap";
  import { formatTime, resumeSeek, shouldSavePosition } from "../../lib/logic/time";
  import SeekBar from "./SeekBar.svelte";

  interface Props {
    src: string;
    /** Saved position for this lesson (seconds). */
    resumeAt: number;
    autoplay: boolean;
    theatre: boolean;
    onsave: (seconds: number) => void;
    onwatched: () => void;
    onended: () => void;
    onerror: (code: number) => void;
    onnext: () => void;
    onprev: () => void;
    ontheatre: () => void;
  }
  let { src, resumeAt, autoplay, theatre, onsave, onwatched, onended, onerror, onnext, onprev, ontheatre }: Props =
    $props();

  let stage: HTMLElement | undefined = $state();
  let video: HTMLVideoElement | undefined = $state();
  let paused = $state(true);
  let current = $state(0);
  let duration = $state(0);
  let buffered = $state(0);
  let volume = $state(1);
  let muted = $state(false);
  let speed = $state(1);
  let fullscreen = $state(false);
  let idle = $state(false);
  let speedOpen = $state(false);
  let flash = $state<string | null>(null);

  // Resume is applied once metadata is known; until then nothing is saved so
  // a fresh element cannot overwrite the stored position with 0.
  let ready = $state(false);
  let lastSave = 0;
  let watchedSent = false;
  let idleTimer: ReturnType<typeof setTimeout> | undefined;
  let flashTimer: ReturnType<typeof setTimeout> | undefined;
  const pipSupported = typeof document !== "undefined" && "pictureInPictureEnabled" in document && document.pictureInPictureEnabled;

  function save(force: boolean) {
    if (!ready || !video) return;
    const now = Date.now();
    if (!shouldSavePosition(now, lastSave, force)) return;
    lastSave = now;
    onsave(video.currentTime);
  }

  function onMeta() {
    if (!video) return;
    duration = video.duration;
    const seek = resumeSeek(resumeAt, video.duration);
    if (seek) video.currentTime = seek;
    ready = true;
    lastSave = Date.now();
    if (autoplay) void video.play().catch(() => {});
  }

  function onTime() {
    if (!video) return;
    current = video.currentTime;
    if (video.buffered.length) buffered = video.buffered.end(video.buffered.length - 1);
    save(false);
    if (!watchedSent && duration && current / duration > 0.9) {
      watchedSent = true;
      onwatched();
    }
  }

  function togglePlay() {
    if (!video) return;
    if (video.paused) void video.play().catch(() => {});
    else video.pause();
  }

  function seekBy(by: number) {
    if (!video || !duration) return;
    video.currentTime = Math.max(0, Math.min(duration, video.currentTime + by));
    show(by > 0 ? `+${by} s` : `${by} s`);
  }

  function setVolume(v: number): number {
    const next = Math.round(Math.max(0, Math.min(1, v)) * 100) / 100;
    if (video) {
      video.volume = next;
      video.muted = next === 0;
    }
    return next;
  }

  function setSpeed(s: number) {
    if (!video) return;
    video.playbackRate = s;
    speed = s;
    speedOpen = false;
    show(`${s}×`);
  }

  async function toggleFullscreen() {
    if (!stage) return;
    try {
      if (document.fullscreenElement) await document.exitFullscreen();
      else await stage.requestFullscreen();
    } catch {
      /* not allowed here */
    }
  }

  async function togglePip() {
    if (!video || !pipSupported) return;
    try {
      if (document.pictureInPictureElement) await document.exitPictureInPicture();
      else await video.requestPictureInPicture();
    } catch {
      /* not available */
    }
  }

  function show(text: string) {
    flash = text;
    clearTimeout(flashTimer);
    flashTimer = setTimeout(() => (flash = null), 700);
  }

  function wake() {
    idle = false;
    clearTimeout(idleTimer);
    idleTimer = setTimeout(() => (idle = true), 2600);
  }

  function run(action: PlayerAction) {
    switch (action.type) {
      case "toggle-play":
        return togglePlay();
      case "seek":
        return seekBy(action.by);
      case "volume":
        return show(`${Math.round(setVolume((video?.volume ?? 1) + action.by) * 100)}%`);
      case "toggle-mute":
        if (video) video.muted = !video.muted;
        return;
      case "toggle-fullscreen":
        return void toggleFullscreen();
      case "toggle-theatre":
        return ontheatre();
      case "speed":
        return setSpeed(nextSpeed(speed, action.dir));
      case "next":
        return onnext();
      case "prev":
        return onprev();
    }
  }

  function isEditable(el: EventTarget | null): boolean {
    if (!(el instanceof HTMLElement)) return false;
    if (el.isContentEditable) return true;
    if (el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement) return true;
    return el instanceof HTMLInputElement && el.type !== "range" && el.type !== "checkbox";
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape" && speedOpen) {
      speedOpen = false;
      return;
    }
    const target = e.target as HTMLElement | null;
    // Let buttons keep Space/Enter for themselves.
    if ((e.key === " " || e.key === "Enter") && target?.closest("button, a")) return;
    const action = actionForKey({
      key: e.key,
      shiftKey: e.shiftKey,
      ctrlKey: e.ctrlKey,
      altKey: e.altKey,
      metaKey: e.metaKey,
      editable: isEditable(e.target),
    });
    if (!action) return;
    e.preventDefault();
    wake();
    run(action);
  }

  function onFullscreenChange() {
    fullscreen = document.fullscreenElement === stage;
  }

  onDestroy(() => {
    save(true);
    clearTimeout(idleTimer);
    clearTimeout(flashTimer);
  });
</script>

<svelte:window onkeydown={onKey} onbeforeunload={() => save(true)} />
<svelte:document onfullscreenchange={onFullscreenChange} />

<div
  class="stage"
  class:theatre
  class:idle={idle && !paused && !speedOpen}
  bind:this={stage}
  onpointermove={wake}
  role="region"
  aria-label={t("play")}
>
  <!-- svelte-ignore a11y_media_has_caption -->
  <video
    bind:this={video}
    {src}
    preload="metadata"
    playsinline
    onloadedmetadata={onMeta}
    ontimeupdate={onTime}
    onprogress={onTime}
    onplay={() => (paused = false)}
    onpause={() => {
      paused = true;
      save(true);
    }}
    onvolumechange={() => {
      volume = video?.volume ?? 1;
      muted = video?.muted ?? false;
    }}
    onratechange={() => (speed = video?.playbackRate ?? 1)}
    onended={() => {
      save(true);
      if (!watchedSent) onwatched();
      watchedSent = true;
      onended();
    }}
    onerror={() => onerror(video?.error?.code ?? 0)}
    onclick={togglePlay}
    ondblclick={() => void toggleFullscreen()}
  ></video>

  {#if paused && ready}
    <button class="big-play" type="button" onclick={togglePlay} aria-label={t("play")}>
      <Icon name="play" size={28} />
    </button>
  {/if}

  {#if flash}<span class="flash num" aria-hidden="true">{flash}</span>{/if}

  <div class="controls">
    <SeekBar {current} {duration} {buffered} onseek={(s) => video && (video.currentTime = s)} />
    <div class="bar">
      <button class="icon-btn" type="button" onclick={togglePlay} aria-label={t(paused ? "play" : "pause")}>
        <Icon name={paused ? "play" : "pause"} />
      </button>
      <button class="icon-btn" type="button" onclick={() => seekBy(-10)} aria-label={t("back10")}>
        <Icon name="back10" />
      </button>
      <button class="icon-btn" type="button" onclick={() => seekBy(10)} aria-label={t("fwd10")}>
        <Icon name="fwd10" />
      </button>
      <div class="volume">
        <button
          class="icon-btn"
          type="button"
          onclick={() => video && (video.muted = !video.muted)}
          aria-label={t(muted ? "unmute" : "mute")}
        >
          <Icon name={muted || volume === 0 ? "mute" : "volume"} />
        </button>
        <input
          type="range"
          min="0"
          max="1"
          step="0.05"
          value={muted ? 0 : volume}
          aria-label={t("volume")}
          aria-valuetext="{Math.round((muted ? 0 : volume) * 100)}%"
          oninput={(e) => setVolume(Number(e.currentTarget.value))}
          style:--v="{(muted ? 0 : volume) * 100}%"
        />
      </div>
      <span class="time num">
        <span>{formatTime(current)}</span><span class="sep">/</span><span class="dur">{formatTime(duration)}</span>
      </span>

      <span class="spacer"></span>

      <div class="speed">
        <button
          class="speed-btn num"
          type="button"
          aria-haspopup="menu"
          aria-expanded={speedOpen}
          aria-label="{t('speed')}: {speed}×"
          onclick={() => (speedOpen = !speedOpen)}
        >
          {speed}×
        </button>
        {#if speedOpen}
          <ul class="menu" role="menu" aria-label={t("speed")}>
            {#each SPEEDS as s (s)}
              <li role="none">
                <button
                  type="button"
                  role="menuitemradio"
                  aria-checked={s === speed}
                  class="num"
                  onclick={() => setSpeed(s)}
                >
                  {s}×
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </div>
      <button
        class="icon-btn"
        type="button"
        onclick={ontheatre}
        aria-pressed={theatre}
        aria-label={t("theatre")}
        title="{t('theatre')} (T)"
      >
        <Icon name="theatre" />
      </button>
      {#if pipSupported}
        <button class="icon-btn" type="button" onclick={togglePip} aria-label={t("pip")} title={t("pip")}>
          <Icon name="pip" />
        </button>
      {/if}
      <button
        class="icon-btn"
        type="button"
        onclick={toggleFullscreen}
        aria-label={t(fullscreen ? "exit_fullscreen" : "fullscreen")}
        title="{t('fullscreen')} (F)"
      >
        <Icon name={fullscreen ? "exitFullscreen" : "fullscreen"} />
      </button>
    </div>
  </div>
</div>

<style>
  .stage {
    position: relative;
    background: var(--bg);
    aspect-ratio: 16 / 9;
    /* Hug the 16:9 picture so the controls never overhang it. */
    width: min(100%, calc((100vh - 168px) * 16 / 9));
    margin-inline: auto;
    overflow: hidden;
    user-select: none;
  }
  .stage.theatre {
    width: min(100%, calc((100vh - 72px) * 16 / 9));
  }
  @media (max-height: 640px) {
    .stage,
    .stage.theatre {
      width: min(100%, calc(52vh * 16 / 9));
    }
    .stage.theatre {
      width: min(100%, calc(72vh * 16 / 9));
    }
    .big-play {
      width: 56px;
      height: 56px;
    }
  }
  .stage:fullscreen {
    width: 100%;
    aspect-ratio: auto;
  }
  .stage.idle {
    cursor: none;
  }
  video {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: contain;
    background: var(--bg);
  }
  .big-play {
    position: absolute;
    left: 50%;
    top: 50%;
    transform: translate(-50%, -50%);
    width: 76px;
    height: 76px;
    display: grid;
    place-items: center;
    padding-left: 4px;
    border-radius: var(--r-pill);
    color: var(--lumen);
    border: 1px solid var(--lumen);
    background: var(--scrim);
    box-shadow: 0 0 80px 24px var(--lumen-glow);
    transition: background-color var(--dur-2) var(--ease-out);
  }
  .big-play:hover {
    background: var(--surface-2);
  }
  .flash {
    position: absolute;
    top: var(--s-6);
    left: 50%;
    transform: translateX(-50%);
    padding: var(--s-1) var(--s-3);
    border-radius: var(--r-1);
    background: var(--scrim);
    font-size: var(--fs-sm);
  }
  .controls {
    position: absolute;
    inset: auto 0 0 0;
    padding: var(--s-8) var(--s-4) var(--s-2);
    background: linear-gradient(to top, var(--scrim), transparent);
    display: grid;
    gap: var(--s-1);
    transition: opacity var(--dur-3) var(--ease-out);
  }
  .idle .controls {
    opacity: 0;
  }
  .controls:focus-within {
    opacity: 1;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: var(--s-1);
    min-width: 0;
  }
  .bar .icon-btn:hover {
    background: color-mix(in srgb, var(--surface-3) 70%, transparent);
  }
  .bar .icon-btn[aria-pressed="true"] {
    color: var(--lumen);
  }
  .volume {
    display: flex;
    align-items: center;
  }
  .volume input {
    width: 76px;
    height: 18px;
    margin: 0 var(--s-2) 0 var(--s-1);
    -webkit-appearance: none;
    appearance: none;
    background: transparent;
    cursor: pointer;
  }
  .volume input::-webkit-slider-runnable-track {
    height: 2px;
    background: linear-gradient(to right, var(--text) var(--v), var(--line) var(--v));
  }
  .volume input::-webkit-slider-thumb {
    -webkit-appearance: none;
    width: 10px;
    height: 10px;
    margin-top: -4px;
    border-radius: 50%;
    background: var(--text);
  }
  .time {
    font-size: var(--fs-xs);
    color: var(--text);
    white-space: nowrap;
    padding-left: var(--s-2);
  }
  .sep {
    color: var(--text-muted);
    margin: 0 var(--s-1);
  }
  .dur {
    color: var(--text-muted);
  }
  .spacer {
    flex: 1;
  }
  .speed {
    position: relative;
  }
  .speed-btn {
    height: 36px;
    min-width: 48px;
    padding: 0 var(--s-2);
    border-radius: var(--r-2);
    font-size: var(--fs-xs);
    font-weight: 600;
  }
  .speed-btn:hover,
  .speed-btn[aria-expanded="true"] {
    background: color-mix(in srgb, var(--surface-3) 70%, transparent);
  }
  .menu {
    position: absolute;
    bottom: 44px;
    right: 0;
    min-width: 96px;
    padding: var(--s-1);
    background: var(--surface-2);
    border: 1px solid var(--line);
    border-radius: var(--r-2);
  }
  .menu button {
    width: 100%;
    height: 32px;
    padding: 0 var(--s-3);
    text-align: left;
    border-radius: var(--r-1);
    font-size: var(--fs-sm);
    color: var(--text-muted);
  }
  .menu button:hover {
    background: var(--surface-3);
    color: var(--text);
  }
  .menu button[aria-checked="true"] {
    color: var(--lumen);
  }
  @media (max-width: 760px) {
    .volume input,
    .bar > .icon-btn:nth-child(2),
    .bar > .icon-btn:nth-child(3) {
      display: none;
    }
  }
</style>
