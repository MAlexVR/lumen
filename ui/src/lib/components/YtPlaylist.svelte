<script lang="ts">
  // One YouTube playlist as a stacked card; opens the playlist page.
  import { mediaUrl } from "../api";
  import { t, tn } from "../i18n/index.svelte";
  import { representativeLesson } from "../logic/cardPreview";
  import { groupStats } from "../logic/progress";
  import { videoPreviewFor } from "../logic/videos";
  import { app } from "../state/app.svelte";
  import { href } from "../state/router.svelte";
  import type { Group } from "../types";
  import VideoCard from "./VideoCard.svelte";

  let { group, index }: { group: Group; index: number } = $props();
  const course = $derived(group.courses[0]);
  const progress = $derived(app.progressOf(group.id));
  const stats = $derived(groupStats(group, app.watchedOf(group.id)));
  const rep = $derived(course ? representativeLesson(course, progress) : null);
  const still = $derived(rep?.lesson.thumbnail && app.mediaBase ? mediaUrl(app.mediaBase, rep.lesson.thumbnail) : null);
</script>

{#if course}
  <VideoCard
    title={group.title}
    {index}
    href={href({ name: "course", group: group.id, course: course.index })}
    pct={stats.pct}
    hint={`${tn("videos", stats.total)} · ${t("yt_seen_n", { n: stats.done })}`}
    hintState={stats.state === "done" ? "done" : stats.state === "progress" ? "progress" : "new"}
    stack
    preview={rep ? videoPreviewFor(rep.lesson, progress, app.mediaBase) : null}
    {still}
  />
{/if}
