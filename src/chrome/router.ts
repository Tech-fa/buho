import type { Route } from "../contracts/catalog";

let route: Route = { name: "home" };
const listeners = new Set<(next: Route) => void>();

export function getRoute(): Route {
  return route;
}

export function setRoute(next: Route): void {
  route = next;
  for (const listener of listeners) {
    listener(route);
  }
}

export function subscribe(listener: (next: Route) => void): () => void {
  listeners.add(listener);
  listener(route);
  return () => {
    listeners.delete(listener);
  };
}
