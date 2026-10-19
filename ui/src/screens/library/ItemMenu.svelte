<script lang="ts">
  // Discreet "more actions" menu (one item today: delete from this computer).
  import Icon from "../../lib/design/Icon.svelte";
  import { t } from "../../lib/i18n/index.svelte";

  let { label, ondelete, video = false }: { label: string; ondelete: () => void; video?: boolean } = $props();
  let open = $state(false);
  /** Open towards the right when there is no room on the left. */
  let alignLeft = $state(false);
  let button: HTMLButtonElement | undefined = $state();
  let menu: HTMLElement | undefined = $state();
  const uid = $props.id();

  function toggle() {
    open = !open;
    if (open && button) {
      const main = button.closest("main")?.getBoundingClientRect().left ?? 0;
      alignLeft = button.getBoundingClientRect().right - 240 < main;
    }
    if (open) queueMicrotask(() => menu?.querySelector<HTMLElement>("button")?.focus());
  }

  function choose() {
    open = false;
    ondelete();
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape" && open) {
      e.stopPropagation();
      open = false;
      button?.focus();
    }
  }

  function onOutside(e: PointerEvent) {
    if (open && !(e.target as Element).closest(`[data-menu="${uid}"]`)) open = false;
  }
</script>

<svelte:document onpointerdown={onOutside} />

<div class="menu-wrap" data-menu={uid} onkeydown={onKey} role="presentation">
  <button
    type="button"
    class="kebab"
    bind:this={button}
    aria-haspopup="menu"
    aria-expanded={open}
    aria-controls="menu-{uid}"
    aria-label={t("del_menu", { name: label })}
    title={t("del_menu", { name: label })}
    onclick={toggle}
  >
    <Icon name="more" size={18} />
  </button>
  {#if open}
    <ul class="menu" class:left={alignLeft} id="menu-{uid}" role="menu" bind:this={menu}>
      <li role="none">
        <button type="button" role="menuitem" onclick={choose}>
          <Icon name="trash" size={16} />{t(video ? "del_menu_video" : "del_menu_item")}
        </button>
      </li>
    </ul>
  {/if}
</div>

<style>
  .menu-wrap {
    position: relative;
  }
  .kebab {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border-radius: var(--r-2);
    color: var(--text);
    background: var(--scrim);
    transition:
      background-color var(--dur-1) var(--ease-out),
      opacity var(--dur-2) var(--ease-out);
  }
  .kebab:hover,
  .kebab[aria-expanded="true"] {
    background: var(--surface-3);
  }
  .menu {
    position: absolute;
    top: 36px;
    right: 0;
    z-index: 5;
    min-width: 230px;
    padding: var(--s-1);
    background: var(--surface-2);
    border: 1px solid var(--line);
    border-radius: var(--r-2);
  }
  .menu.left {
    right: auto;
    left: 0;
  }
  .menu button {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    width: 100%;
    height: 36px;
    padding: 0 var(--s-3);
    border-radius: var(--r-1);
    font-size: var(--fs-sm);
    color: var(--text);
    text-align: left;
  }
  .menu button:hover,
  .menu button:focus-visible {
    background: var(--surface-3);
  }
  .menu button :global(svg) {
    color: var(--text-muted);
  }
</style>
