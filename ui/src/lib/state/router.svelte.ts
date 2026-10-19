// Reactive wrapper around the hash router.
import { formatRoute, parseRoute, type Route } from "../logic/route";

export const router = $state<{ route: Route }>({ route: parseRoute(location.hash) });

window.addEventListener("hashchange", () => {
  router.route = parseRoute(location.hash);
});

export const href = (route: Route): string => formatRoute(route);

export function navigate(route: Route): void {
  location.hash = formatRoute(route);
}
