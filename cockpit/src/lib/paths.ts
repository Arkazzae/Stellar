/**
 * Where the web client's server mounts the panel: the overview is here, and
 * every other page below it (web/src/app/cockpit/).
 */
export const COCKPIT_ROOT = "/cockpit";

/** The address of a page given by its path inside the panel, `/` for the overview. */
export function cockpitHref(page: string): string {
  return page === "/" ? COCKPIT_ROOT : `${COCKPIT_ROOT}${page}`;
}
