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

  /** Directory of each platform under the library root. */
  const FOLDER_ROOT = { hotmart: "Hotmart", pluralsight: "Pluralsight", goldenlife: "GoldenLife", harbiz: "Harbiz" } as const;

  let { session, platform, items, heading, emptyText }: {
    session: DownloadItemSession;
    platform: "hotmart" | "pluralsight" | "goldenlife" | "harbiz";
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
          folder={`${FOLDER_ROOT[platform]}/${course.folder}`}
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
