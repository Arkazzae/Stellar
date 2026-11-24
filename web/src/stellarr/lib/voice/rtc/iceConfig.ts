import type { IceServerConfig } from "@/lib/api/types";

export interface ResolveIceInput {
  /** `rtc.iceServers` from `/api/bootstrap`, with per-user credentials when TURN is enabled. */
  runtime?: readonly IceServerConfig[] | undefined;
  /** `VITE_ICE_SERVERS`, baked in at build time. */
  env?: readonly RTCIceServer[] | undefined;
  /** False only in development builds. */
  isProduction: boolean;
}

/**
 * Decides which ICE servers a peer connection gets.
 *
 * In production, use the runtime list from `/api/bootstrap`. An empty list is
 * normal for direct media over ICE-UDP or ICE-TCP. When TURN is enabled, the
 * runtime list carries time-limited credentials unique to the user.
 *
 * A development build can fall back to a build-time STUN list when bootstrap
 * supplies no ICE servers.
 */
export function resolveIceServers({
  runtime,
  env,
  isProduction,
}: ResolveIceInput): RTCIceServer[] {
  const fromRuntime = (runtime ?? []).flatMap(toRtcIceServer);
  if (fromRuntime.length > 0) return fromRuntime;
  if (isProduction) return [];
  return (env ?? []).map((server) => ({ ...server }));
}

/**
 * Converts one wire entry. `username` and `credential` pass through verbatim:
 * coturn recomputes the HMAC over the username, so trimming or re-casing either
 * half breaks authentication.
 *
 * An entry with no usable URL is dropped, because `RTCPeerConnection` throws on
 * a malformed `urls` value and that would take down the whole call.
 */
function toRtcIceServer({ urls, username, credential }: IceServerConfig): RTCIceServer[] {
  const usable = (Array.isArray(urls) ? urls : [urls]).filter(
    (url): url is string => typeof url === "string" && url !== "",
  );
  if (usable.length === 0) return [];

  // coturn rejects a username without its HMAC, and a lone credential means
  // nothing, so the pair is attached only when both halves are present.
  const hasCredentials = !!username && !!credential;
  return [hasCredentials ? { urls: usable, username, credential } : { urls: usable }];
}
