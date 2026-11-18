import type { ApiClient } from "../client";
import {
  parseAccountSecurity,
  parseOwnSession,
  parseMfaChallenge,
  parseRecoveryCodes,
  parseSecurityEvent,
} from "../securityParsers";
import { ApiError } from "../errors";
import { expectRecord, expectString } from "../expect";
import { deleteResource, getJson, parseList, route, sendJson, sendVoid } from "./http";

type SecurityCalls = Pick<
  ApiClient,
  | "getAccountSecurity"
  | "listSecurityEvents"
  | "startMfaEnrollment"
  | "startMfaReverification"
  | "removeMfaMethod"
  | "regenerateRecoveryCodes"
  | "revokeTrustedDevice"
  | "listOwnSessions"
  | "revokeOwnSession"
  | "revokeOtherSessions"
  | "startIdentityLink"
  | "removeIdentity"
  | "startReauthentication"
  | "setPassword"
>;

/** Only the server's own link and re-authentication routes are followed. */
const LINK_ADDRESS = /^\/api\/auth\/oidc\/([a-z0-9-]+\/)?link\?ticket=[^\s]+$/;
const REAUTHENTICATION_ADDRESS = /^\/api\/auth\/oidc\/([a-z0-9-]+\/)?reauthenticate\?ticket=[^\s]+$/;

const parseAddress = (route: RegExp) => (value: unknown, path: string): string => {
  const url = expectString(expectRecord(value, path), "url", path);
  if (!route.test(url)) throw new ApiError("malformed", `${path}: not an address of this server's sign-in routes`);
  return url;
};

const withPassword = (password?: string) => (password === undefined ? {} : { password });

/** The signed-in account's second factor: its methods, recovery codes, trusted devices and history. */
export function securityCalls(): SecurityCalls {
  return {
    getAccountSecurity: (signal) => getJson("/api/me/security", parseAccountSecurity, signal),

    listSecurityEvents: (signal) =>
      getJson("/api/me/security/events", parseList(parseSecurityEvent), signal),

    startMfaEnrollment: (password, signal) =>
      sendJson(
        "POST",
        "/api/me/security/mfa/enroll/start",
        withPassword(password),
        parseMfaChallenge,
        signal,
      ),

    startMfaReverification: (signal) =>
      sendJson("POST", "/api/me/security/mfa/reverify/start", {}, parseMfaChallenge, signal),

    removeMfaMethod: (methodId, password) =>
      sendVoid("POST", route`/api/me/security/mfa/${methodId}/remove`, withPassword(password)),

    regenerateRecoveryCodes: (password, signal) =>
      sendJson(
        "POST",
        "/api/me/security/recovery/regenerate",
        withPassword(password),
        parseRecoveryCodes,
        signal,
      ),

    // A device that is already gone is as untrusted as it can be.
    revokeTrustedDevice: (deviceId) => deleteResource(route`/api/me/security/trusted/${deviceId}`),

    listOwnSessions: (signal) =>
      getJson("/api/me/security/sessions", parseList(parseOwnSession), signal),

    // A session that already ended is as signed out as it can be.
    revokeOwnSession: (sessionId) => deleteResource(route`/api/me/security/sessions/${sessionId}`),

    revokeOtherSessions: () => sendVoid("POST", "/api/me/security/sessions/revoke-others"),

    startIdentityLink: (providerId, password) =>
      sendJson(
        "POST",
        route`/api/me/security/identities/${providerId}/link`,
        withPassword(password),
        parseAddress(LINK_ADDRESS),
      ),

    removeIdentity: (identityId, password) =>
      sendVoid("POST", route`/api/me/security/identities/${identityId}/remove`, withPassword(password)),

    startReauthentication: (providerId) =>
      sendJson(
        "POST",
        route`/api/me/security/identities/${providerId}/reauthenticate`,
        {},
        parseAddress(REAUTHENTICATION_ADDRESS),
      ),

    setPassword: (newPassword) => sendVoid("POST", "/api/me/security/password", { newPassword }),
  };
}
