import type { CockpitLoginStep } from "./api";
import { COCKPIT_ROOT } from "./paths";

/*
 * The web client's sign-in page, on the same origin and from the same
 * server, so the page shares Cockpit's session and comes back here once the
 * step is done.
 */
const BACK_TO_COCKPIT = `returnUrl=${encodeURIComponent(COCKPIT_ROOT)}`;

/**
 * Where a sign-in the password did not finish goes on; the page resumes the
 * pending step. A challenge already holds the keep-signed-in choice on the
 * server; an operator's recovery has none yet, so the address carries it.
 */
export function webSignInUrl(step: CockpitLoginStep, keepSignedIn = true): string {
  if (step !== "security_recovery_required") return `/login?${BACK_TO_COCKPIT}`;
  const recovery = `/login?${BACK_TO_COCKPIT}&security_recovery_required=1`;
  return keepSignedIn ? recovery : `${recovery}&keep_signed_in=0`;
}

/** Where a signed-in owner confirms a second step, or sets up a first method, for Cockpit. */
export const WEB_CONFIRM_URL = `/login?confirm=1&${BACK_TO_COCKPIT}`;

/**
 * Where an owner confirms a second step from the last few minutes before a
 * sensitive change, coming back to the page that asked for it.
 */
export function webFreshConfirmUrl(returnPath: string): string {
  return `/login?confirm=1&fresh=1&returnUrl=${encodeURIComponent(returnPath)}`;
}

/** A full page load: the sign-in page renders in the web client's layout, not in this panel's. */
export function leaveFor(path: string): void {
  window.location.assign(path);
}
