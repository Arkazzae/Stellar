/**
 * Where the panel's routes are mounted: the overview is here, and every other
 * page below it (src/app/(cockpit)/cockpit/).
 */
export const COCKPIT_ROOT = "/cockpit";

/** The address of a page given by its path inside the panel, `/` for the overview. */
export function cockpitHref(page: string): string {
  return page === "/" ? COCKPIT_ROOT : `${COCKPIT_ROOT}${page}`;
}
