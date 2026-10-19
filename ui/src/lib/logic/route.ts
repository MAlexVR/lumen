// Tiny hash router: `#/screen/arg/arg`, each arg URI-encoded.

export type Route =
  | { name: "home" }
  | { name: "library"; platform: string | null }
  | { name: "course"; group: string; course: number }
  | { name: "watch"; group: string; key: string }
  | { name: "settings" }
  | { name: "downloads"; platform: string | null };

const HOME: Route = { name: "home" };

export function parseRoute(hash: string): Route {
  let parts: string[];
  try {
    parts = hash
      .replace(/^#\/?/, "")
      .split("/")
      .filter(Boolean)
      .map(decodeURIComponent);
  } catch {
    return HOME;
  }
  const [name, a, b] = parts;
  switch (name) {
    case "library":
      return { name: "library", platform: a ?? null };
    case "settings":
      return { name };
    case "downloads":
    case "descargas":
      return { name: "downloads", platform: a ?? null };
    case "course": {
      const course = Number(b);
      return a && Number.isInteger(course) && b !== "" ? { name: "course", group: a, course } : HOME;
    }
    case "watch":
      return a && b ? { name: "watch", group: a, key: b } : HOME;
    default:
      return HOME;
  }
}

const enc = encodeURIComponent;

export function formatRoute(route: Route): string {
  switch (route.name) {
    case "home":
      return "#/";
    case "library":
      return route.platform ? `#/library/${enc(route.platform)}` : "#/library";
    case "course":
      return `#/course/${enc(route.group)}/${route.course}`;
    case "watch":
      return `#/watch/${enc(route.group)}/${enc(route.key)}`;
    case "downloads":
      return route.platform ? `#/downloads/${enc(route.platform)}` : "#/downloads";
    default:
      return `#/${route.name}`;
  }
}
