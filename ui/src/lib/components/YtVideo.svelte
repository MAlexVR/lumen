<script lang="ts">
  // One standalone YouTube video as a card; opens the player on it.
  import { mediaUrl } from "../api";
  import { t } from "../i18n/index.svelte";
  import { positionOf } from "../logic/progress";
  import { formatTime } from "../logic/time";
  import { videoPreviewFor, videoState, type VideoItem } from "../logic/videos";
  import { app } from "../state/app.svelte";
  import { href } from "../state/router.svelte";
  import VideoCard from "./VideoCard.svelte";

  let { item, index }: { item: VideoItem; index: number } = $props();
  const progress = $derived(app.progressOf(item.group.id));
  const seconds = $derived(positionOf(progress, item.lesson.key));
  const watched = $derived(app.watchedOf(item.group.id).has(item.lesson.key));
  const state = $derived(videoState(watched, seconds));
  const hint = $derived(
    state === "done" ? t("yt_video_seen") : state === "progress" ? t("yt_video_resume", { time: formatTime(seconds) }) : t("yt_video_new"),
  );
  const still = $derived(item.lesson.thumbnail && app.mediaBase ? mediaUrl(app.mediaBase, item.lesson.thumbnail) : null);
</script>

<VideoCard
  title={item.lesson.title}
  {index}
  href={href({ name: "watch", group: item.group.id, key: item.lesson.key })}
  pct={watched ? 100 : 0}
  {hint}
  hintState={state}
  preview={videoPreviewFor(item.lesson, progress, app.mediaBase)}
  {still}
/>
