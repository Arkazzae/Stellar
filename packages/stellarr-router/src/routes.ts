import type { ComponentType } from "react";

import { canonicalPath } from "./address";

/** A page: a component that takes no props and reads the address through the hooks. */
export type Page = ComponentType;

export interface Route {
  /**
   * The address, from the root: `/login`, `/invite/:code`. A `:name` segment
   * matches one segment of any value, read with `useParams`. No trailing
   * slash, except for the root itself.
   */
  readonly path: string;
  readonly page: Page;
}

/** A parameter's decoded value, or null when its escapes are malformed. */
export type Params = Readonly<Record<string, string | null>>;

export interface RouteMatch {
  readonly page: Page;
  readonly params: Params;
  /** The route's `path`, or null when the not-found page answers. */
  readonly route: string | null;
}

export interface RouteTable {
  /** The page for a path, the not-found page when no route claims it. */
  match(pathname: string): RouteMatch;
  /**
   * Whether a route claims the path. The not-found page claims nothing, so
   * an address only it would answer, such as another app's, is left to a
   * page load.
   */
  claims(pathname: string): boolean;
}

type Segment = { readonly kind: "static"; readonly value: string } | { readonly kind: "param"; readonly name: string };

interface CompiledRoute {
  readonly path: string;
  readonly page: Page;
  readonly segments: readonly Segment[];
}

function segmentsOf(pathname: string): string[] {
  return pathname === "/" ? [] : pathname.slice(1).split("/");
}

/** `decodeURIComponent`, or null for an escape it cannot read. */
function decode(segment: string): string | null {
  try {
    return decodeURIComponent(segment);
  } catch {
    return null;
  }
}

function compile(route: Route): CompiledRoute {
  const { path } = route;
  if (!path.startsWith("/") || canonicalPath(path) !== path || path.includes("//")) {
    throw new Error(`A route's path starts with / and has no empty segment: ${JSON.stringify(path)}`);
  }
  const names = new Set<string>();
  const segments = segmentsOf(path).map((part): Segment => {
    if (!part.startsWith(":")) return { kind: "static", value: part };
    const name = part.slice(1);
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(name) || names.has(name)) {
      throw new Error(`A route's parameter needs a name of its own: ${JSON.stringify(path)}`);
    }
    names.add(name);
    return { kind: "param", name };
  });
  return { path, page: route.page, segments };
}

function matchRoute(route: CompiledRoute, parts: readonly string[]): Params | null {
  if (route.segments.length !== parts.length) return null;
  const params: Record<string, string | null> = {};
  for (let index = 0; index < parts.length; index += 1) {
    const segment = route.segments[index];
    const part = parts[index] ?? "";
    if (segment === undefined || part === "") return null;
    if (segment.kind === "param") params[segment.name] = decode(part);
    else if ((decode(part) ?? part) !== segment.value) return null;
  }
  return params;
}

const NO_PARAMS: Params = Object.freeze({});

/**
 * The table of an app's addresses. The first route whose path matches
 * answers; any other address gets `notFound`. Paths are compared in their
 * canonical form, so `/login/` is `/login`.
 */
export function defineRoutes(table: { readonly routes: readonly Route[]; readonly notFound: Page }): RouteTable {
  const compiled = table.routes.map(compile);
  const seen = new Set<string>();
  for (const route of compiled) {
    const shape = route.segments.map((segment) => (segment.kind === "param" ? ":" : segment.value)).join("/");
    if (seen.has(shape)) throw new Error(`Two routes claim the same addresses: ${JSON.stringify(route.path)}`);
    seen.add(shape);
  }

  function find(pathname: string): RouteMatch | null {
    const parts = segmentsOf(canonicalPath(pathname));
    for (const route of compiled) {
      const params = matchRoute(route, parts);
      if (params !== null) return { page: route.page, params: Object.freeze(params), route: route.path };
    }
    return null;
  }

  return {
    match: (pathname) => find(pathname) ?? { page: table.notFound, params: NO_PARAMS, route: null },
    claims: (pathname) => find(pathname) !== null,
  };
}

function Nothing() {
  return null;
}

/** A table that claims nothing and shows nothing, for a router given none. */
export const NO_ROUTES: RouteTable = defineRoutes({ routes: [], notFound: Nothing });
