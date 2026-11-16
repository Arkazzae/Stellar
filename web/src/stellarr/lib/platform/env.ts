/**
 * ICE servers for instances that do not run coturn, read from a JSON array:
 *
 *   VITE_ICE_SERVERS='[{"urls":"stun:stun.example.org:3478"}]'
 *
 * The per-user TURN credentials in `/api/bootstrap` take precedence over these.
 * Anything that is not a JSON array yields no servers rather than an error.
 */
export function iceServers(): RTCIceServer[] {
  // Written into the bundle when it is built; the browser has no environment.
  const raw = import.meta.env.VITE_ICE_SERVERS;
  if (!raw) return [];
  try {
    const parsed: unknown = JSON.parse(raw);
    return Array.isArray(parsed) ? (parsed as RTCIceServer[]) : [];
  } catch {
    return [];
  }
}
