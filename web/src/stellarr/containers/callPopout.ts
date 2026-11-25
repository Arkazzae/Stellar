import { useEffect } from "react";

import type { Channel } from "@/ports";

const HANDOFF = "stellarr:call-handoff";
const HANDOFF_MS = 60_000;
let launchConsumed = false;

export function popOutCall(channelId: string, serverId: string): void {
  if (serverId.length === 0) return;
  const token = crypto.randomUUID();
  try {
    window.sessionStorage.setItem(
      HANDOFF,
      JSON.stringify({ token, channelId, serverId, expires: Date.now() + HANDOFF_MS }),
    );
  } catch {
    return;
  }
  const url = new URL(window.location.href);
  url.searchParams.set("popout", "1");
  url.searchParams.set("voiceChannel", channelId);
  url.searchParams.set("callHandoff", token);
  // A named window can already exist on another server. Its session storage
  // is independent, so the launch address must carry the target explicitly.
  url.searchParams.set("serverId", serverId);
  window.open(url, "stellarr-call", "popup,width=1100,height=760");
}

/** A popout can dial only a handoff authorized by its same-origin opener. */
export function useJoinFromPopoutLink(
  join: (channelId: string) => void,
  serverId: string,
  channels: readonly Pick<Channel, "id" | "kind">[],
): void {
  useEffect(() => {
    if (launchConsumed) return;
    const url = new URL(window.location.href);
    const params = url.searchParams;
    const channelId = params.get("voiceChannel");
    const token = params.get("callHandoff");
    if (params.get("popout") !== "1" || !channelId || !token || !window.opener) {
      launchConsumed = true;
      return;
    }
    // The directory may still be loading the server named in the handoff.
    if (
      serverId !== params.get("serverId") ||
      !channels.some((channel) => channel.id === channelId && channel.kind === "voice")
    ) return;
    launchConsumed = true;
    try {
      const opener = window.opener as Window;
      const storage = opener.sessionStorage;
      const stored = storage.getItem(HANDOFF);
      if (!stored) return;
      const handoff = JSON.parse(stored);
      if (
        handoff.token !== token || handoff.channelId !== channelId ||
        handoff.serverId !== serverId || !(Date.now() < handoff.expires)
      ) return;
      storage.removeItem(HANDOFF);
    } catch {
      // Cross-origin openers and unavailable storage cannot authorize a call.
      return;
    }
    params.delete("voiceChannel");
    params.delete("callHandoff");
    window.history.replaceState(window.history.state, "", url);
    join(channelId);
  }, [join, serverId, channels]);
}
