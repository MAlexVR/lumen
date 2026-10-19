<script lang="ts">
  import { displayTitle } from "../../lib/logic/title";
  // "Continue where you left off": editorial hero with the lesson as headline.
  import Icon from "../../lib/design/Icon.svelte";
  import PosterArt from "../../lib/components/PosterArt.svelte";
  import ProgressLine from "../../lib/components/ProgressLine.svelte";
  import { t } from "../../lib/i18n/index.svelte";
  import { courseStats, flattenCourse, type ContinueTarget } from "../../lib/logic/progress";
  import { previewFace, previewSrc, previewTime } from "../../lib/logic/preview";
  import { heroSource } from "../../lib/logic/thumbs";
  import { thumbAt, thumbs } from "../../lib/state/thumbs.svelte";
  import { formatTime, RESUME_MARGIN } from "../../lib/logic/time";
  import { isYoutube, wording } from "../../lib/logic/videos";
  import { app } from "../../lib/state/app.svelte";
  import { href } from "../../lib/state/router.svelte";
  import type { Group } from "../../lib/types";

  interface Props {
    group: Group;
    target: ContinueTarget;
    watched: ReadonlySet<string>;
  }
  let { group, target, watched }: Props = $props();

  const wd = $derived(wording(group.platform));
  const yt = $derived(isYoutube(group.platform));
  const loose = $derived(group.collection === "standalone");
  const upHref = $derived(
    loose ? href({ name: "library", platform: "youtube" }) : href({ name: "course", group: group.id, course: target.item.course.index }),
  );
  const item = $derived(target.item);
  const cStats = $derived(courseStats(item.course, watched));
  const courseFlat = $derived(flattenCourse(item.course));
  const position = $derived(courseFlat.findIndex((f) => f.lesson.key === item.lesson.key) + 1);
  const resume = $derived(target.kind === "resume");
  const showResume = $derived(resume && target.seconds >= RESUME_MARGIN);
  const isWatched = $derived(watched.has(item.lesson.key));

  // Preview: a paused, muted frame of the real video over the generated
  // poster, shown only once the chosen frame is decoded (never autoplays).
  let video: HTMLVideoElement | undefined = $state();
  let ready = $state(false);
  let failed = $state(false);
  let duration = $state(0);
  const src = $derived(previewSrc(app.mediaBase, item.lesson));
  // The cached thumbnail is the still; the video only backs it up when
  // thumbnails are unavailable.
  let imgLoaded = $state(false);
  const thumbItem = $derived(src && item.lesson.video ? { path: item.lesson.video, atSecs: thumbAt(target.seconds) } : null);
  const lookup = $derived(thumbItem ? thumbs.lookup(thumbItem) : ({ state: "idle" } as const));
  const source = $derived(heroSource(lookup));
  const face = $derived(imgLoaded && source === "thumb" ? "video" : previewFace({ src: source === "video" ? src : null, ready, failed }));

  $effect(() => {
    const it = thumbItem;
    if (!it) return;
    thumbs.want(it);
    return () => thumbs.leave(it);
  });

  $effect(() => {
    void src;
    ready = false;
    failed = false;
    imgLoaded = false;
    duration = 0;
  });

  function onMeta() {
    if (!video) return;
    duration = video.duration;
    const at = previewTime(target.seconds, duration);
    if (at > 0) video.currentTime = at;
  }

  function onFrame() {
    // loadeddata covers a frame at 0; seeked covers the chosen frame.
    if (video && (video.currentTime > 0 || previewTime(target.seconds, duration) === 0)) ready = true;
  }
</script>

<section class="hero" aria-labelledby="hero-title">
  <div class="text">
    <span class="eyebrow lead">{t(resume ? "continue_eyebrow" : "start_eyebrow")}</span>
    <p class="crumb">
      <a href={upHref}>{loose ? t("videos_title") : displayTitle(item.course.title)}</a>
      {#if !yt}
        <Icon name="chevronRight" size={14} />
        <span>{displayTitle(item.section.title)}</span>
      {/if}
    </p>
    {#if group.seller}<p class="seller">{group.seller}</p>{/if}
    <h1 id="hero-title">{displayTitle(item.lesson.title)}</h1>

    <dl class="facts">
      <div>
        <dt class="eyebrow">{t(wd.title)}</dt>
        <dd class="num">{t(wd.of, { n: position, total: courseFlat.length })}</dd>
      </div>
      {#if showResume}
        <div>
          <dt class="eyebrow">{t("resume_at")}</dt>
          <dd class="num time">{formatTime(target.seconds)}</dd>
        </div>
      {/if}
      <div>
        <dt class="eyebrow">{t("course_progress")}</dt>
        <dd>
          <span class="num">{cStats.pct}%</span>
          <span class="muted">· {t(`state_${cStats.state}` as const)}</span>
        </dd>
      </div>
    </dl>

    <ProgressLine value={cStats.pct} label={displayTitle(item.course.title)} />

    <div class="actions">
      <a class="btn btn-primary" href={href({ name: "watch", group: group.id, key: item.lesson.key })}>
        <Icon name="play" size={16} />
        {t(resume ? "btn_continue" : "btn_start")}
      </a>
      <a class="btn btn-quiet" href={upHref}>
        {t(!yt ? "view_course" : loose ? "yt_view_videos" : "yt_view_list")}
      </a>
      {#if isWatched}
        <span class="watched"><Icon name="check" size={16} />{t("watched")}</span>
      {/if}
    </div>
  </div>

  <a class="art" href={href({ name: "watch", group: group.id, key: item.lesson.key })} tabindex="-1" aria-hidden="true">
    <span class="halo"></span>
    <span class="frame" data-face={face}>
      <PosterArt title={item.course.title} index={item.course.index} shape="still" glow />
      {#if lookup.state === "ready"}
        <img class="preview" class:shown={imgLoaded} src={lookup.url} alt="" decoding="async" draggable="false" onload={() => (imgLoaded = true)} />
      {/if}
      {#if source === "video" && src && !failed}
        <video
          bind:this={video}
          class="preview"
          class:shown={face === "video"}
          {src}
          muted
          playsinline
          preload="metadata"
          disablepictureinpicture
          disableremoteplayback
          tabindex="-1"
          aria-hidden="true"
          onloadedmetadata={onMeta}
          onloadeddata={onFrame}
          onseeked={onFrame}
          onplay={() => video?.pause()}
          onerror={() => (failed = true)}
        ></video>
      {/if}
      <span class="play"><Icon name="play" size={22} /></span>
      {#if showResume}
        <span class="stamp num">{formatTime(target.seconds)}</span>
      {:else if face === "video" && source === "video" && duration > 0}
        <span class="stamp num">{formatTime(duration)}</span>
      {/if}
    </span>
  </a>
</section>

<style>
  .hero {
    position: relative;
    display: grid;
    grid-template-columns: minmax(0, 1.05fr) minmax(0, 1fr);
    gap: var(--s-12);
    align-items: center;
    padding: var(--s-16) var(--gutter) var(--s-4);
    min-height: min(64vh, 560px);
  }
  .text {
    display: grid;
    gap: var(--s-5);
    min-width: 0;
  }
  .lead {
    color: var(--lumen);
  }
  .crumb {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    color: var(--text-muted);
    font-size: var(--fs-sm);
    min-width: 0;
  }
  .seller {
    color: var(--text-muted);
    font-size: var(--fs-sm);
    margin-top: calc(-1 * var(--s-3));
  }
  .crumb a {
    color: var(--text);
    text-decoration: none;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .crumb a:hover {
    text-decoration: underline;
  }
  .crumb span {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  h1 {
    /* Sora is wide: the two-column hero needs a step below --fs-display. */
    font-size: clamp(2rem, 0.9rem + 2.9vw, 3.75rem);
    line-height: 1.06;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
    padding-bottom: 0.06em;
  }
  .facts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-3) var(--s-10);
    margin: var(--s-2) 0 0;
  }
  .facts div {
    display: grid;
    gap: 2px;
  }
  dd {
    margin: 0;
    font-size: var(--fs-md);
  }
  .time {
    color: var(--lumen);
  }
  .muted {
    color: var(--text-muted);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-3);
    margin-top: var(--s-3);
  }
  .actions a {
    text-decoration: none;
  }
  .watched {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    color: var(--ok);
    font-size: var(--fs-sm);
    margin-left: var(--s-2);
  }
  .art {
    position: relative;
    display: block;
  }
  .halo {
    position: absolute;
    inset: -22% -14%;
    background: radial-gradient(closest-side, var(--lumen-glow), transparent 72%);
    pointer-events: none;
  }
  .frame {
    position: relative;
    display: block;
    aspect-ratio: 16 / 9;
    border-radius: var(--r-2);
    overflow: hidden;
    outline: 1px solid var(--line);
    outline-offset: -1px;
    transition: filter var(--dur-2) var(--ease-out);
  }
  .art:hover .frame[data-face="poster"] {
    filter: brightness(1.3);
  }
  .preview {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    background: var(--bg);
    opacity: 0;
    transition:
      opacity var(--dur-3) var(--ease-out),
      filter var(--dur-2) var(--ease-out);
    pointer-events: none;
  }
  .preview.shown {
    opacity: 1;
  }
  /* Real footage can be bright: seat it on the black stage, lift on hover. */
  .preview {
    filter: brightness(0.82);
  }
  .art:hover .preview {
    filter: brightness(1);
  }
  .frame[data-face="video"]::after {
    content: "";
    position: absolute;
    inset: 55% 0 0 0;
    background: linear-gradient(to top, var(--scrim), transparent);
    pointer-events: none;
  }
  .play {
    z-index: 1;
    position: absolute;
    left: var(--s-5);
    bottom: var(--s-5);
    display: grid;
    place-items: center;
    width: 52px;
    height: 52px;
    border-radius: var(--r-pill);
    border: 1px solid var(--lumen);
    color: var(--lumen);
    background: var(--scrim);
    padding-left: 3px;
  }
  .stamp {
    z-index: 1;
    position: absolute;
    right: var(--s-4);
    bottom: var(--s-4);
    font-size: var(--fs-xs);
    color: var(--text);
    background: var(--scrim);
    padding: 2px var(--s-2);
    border-radius: var(--r-1);
  }

  @media (max-width: 980px) {
    .hero {
      grid-template-columns: 1fr;
      gap: var(--s-8);
      padding-top: var(--s-10);
      min-height: 0;
    }
    .art {
      display: none;
    }
  }
</style>
