<script lang="ts" module>
  // Hand-drawn 24px icon set: stroke 1.5, square caps, mitre joins.
  const PATHS = {
    home: "M4 11 12 4.5 20 11M6.5 9v10.5h11V9M10 19.5v-5h4v5",
    library: "M4.5 4.5h3v15h-3zM10.5 4.5h3v15h-3zM16 5.2l2.9-.8 3.6 14.4-2.9.8z",
    download: "M12 4v11M7.5 10.5 12 15l4.5-4.5M5 19.5h14",
    settings:
      "M4 7h9M17 7h3M4 17h3M11 17h9M15 4.5v5M9 14.5v5",
    play: "M7.5 5v14L19 12z",
    pause: "M7.5 5v14M16.5 5v14",
    prev: "M6.5 5.5v13M18 5.5 9.5 12l8.5 6.5z",
    next: "M17.5 5.5v13M6 5.5l8.5 6.5L6 18.5z",
    restart: "M6.5 5v14M19 5.5v13L9.5 12z",
    back10: "M4.5 12a7.5 7.5 0 1 0 2.2-5.3M4.5 4.5v3.2h3.2",
    fwd10: "M19.5 12a7.5 7.5 0 1 1-2.2-5.3M19.5 4.5v3.2h-3.2",
    volume: "M4.5 9.5h3l4.5-4v13l-4.5-4h-3zM15.5 9a4 4 0 0 1 0 6M18 6.5a7.5 7.5 0 0 1 0 11",
    mute: "M4.5 9.5h3l4.5-4v13l-4.5-4h-3zM16 9.5l5 5M21 9.5l-5 5",
    theatre: "M3.5 6.5h17v11h-17z",
    fullscreen: "M4.5 9V4.5H9M15 4.5h4.5V9M19.5 15v4.5H15M9 19.5H4.5V15",
    exitFullscreen: "M9 4.5V9H4.5M19.5 9H15V4.5M15 19.5V15h4.5M4.5 15H9v4.5",
    pip: "M3.5 5.5h17v13h-17zM12.5 12h6v4.5h-6z",
    check: "M5 12.5 9.5 17 19 7.5",
    chevronRight: "M9.5 5.5 16 12l-6.5 6.5",
    chevronDown: "M5.5 9.5 12 16l6.5-6.5",
    arrowLeft: "M19.5 12h-15M10.5 6l-6 6 6 6",
    search: "M10.5 4.5a6 6 0 1 0 0 12 6 6 0 0 0 0-12zM15 15l5 5",
    file: "M6.5 3.5h7l4 4v13h-11zM13.5 3.5v4h4",
    link: "M10 14 14 10M8.5 11.5l-2 2a3 3 0 0 0 4.2 4.2l2-2M15.5 12.5l2-2a3 3 0 0 0-4.2-4.2l-2 2",
    external: "M13.5 4.5h6v6M19.5 4.5 11 13M17.5 14v5.5h-13v-13H10",
    list: "M4.5 6.5h15M4.5 12h15M4.5 17.5h15",
    close: "M6 6l12 12M18 6 6 18",
    noVideo: "M3.5 6.5h12v11h-12zM15.5 10l5-3v10l-5-3M3 3l18 18",
    alert: "M12 4 21 19.5H3zM12 10v4.5M12 16.5v1",
    more: "M5.25 11.25h1.5v1.5h-1.5zM11.25 11.25h1.5v1.5h-1.5zM17.25 11.25h1.5v1.5h-1.5z",
    trash: "M4.5 6.5h15M9.5 6.5V4h5v2.5M6.5 6.5l1 13.5h9l1-13.5M10 10.5v6M14 10.5v6",
  } as const;
  export type IconName = keyof typeof PATHS;
</script>

<script lang="ts">
  interface Props {
    name: IconName;
    size?: number;
    /** Accessible name; omit for decorative icons next to visible text. */
    label?: string;
  }
  let { name, size = 20, label }: Props = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.5"
  stroke-linecap="square"
  stroke-linejoin="miter"
  role={label ? "img" : undefined}
  aria-label={label}
  aria-hidden={label ? undefined : "true"}
  focusable="false"
>
  <path d={PATHS[name]} fill={name === "play" ? "currentColor" : "none"} />
</svg>

<style>
  svg {
    flex: none;
    display: block;
  }
</style>
