<script lang="ts">
  import { localStatusByFolder } from "../../lib/logic/localStatus";
  import { app } from "../../lib/state/app.svelte";
  import type { DownloadItemSession } from "../../lib/state/downloads.svelte";
  import type { PlatformId } from "../../lib/types";
  import ItemRow from "./ItemRow.svelte";

  export interface CourseListItem {
    key: string;
    title: string;
    coverUrl: string | null;
    facts: string[];
    folder: string;
    issue?: string | null;
  }

  let { session, platform, items, heading, emptyText }: {
    session: DownloadItemSession;
    platform: "hotmart" | "pluralsight";
    items: CourseListItem[];
    heading: string;
    emptyText: string;
  } = $props();
</script>

<section aria-label={heading}>
  <h2 class="eyebrow section">{heading}</h2>
  {#if items.length}
    <ul class="items">
      {#each items as course (course.key)}
        <ItemRow
          {session}
          {platform}
          itemKey={course.key}
          name={course.title}
          coverUrl={course.coverUrl}
          facts={course.facts}
          local={localStatusByFolder(app.library, platform, course.folder)}
          folder={`${platform === "hotmart" ? "Hotmart" : "Pluralsight"}/${course.folder}`}
          disabledReason={course.issue}
        />
      {/each}
    </ul>
  {:else}
    <p class="muted">{emptyText}</p>
  {/if}
</section>

<style>
  .section {
    font-family: var(--font-ui);
    margin-bottom: var(--s-2);
  }
  .items {
    border-top: 1px solid var(--line);
  }
  .muted {
    color: var(--text-muted);
  }
</style>
