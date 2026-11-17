import { useEffect } from "react";

/**
 * A popped-out call is the same app in a small window, told by its address
 * which call to dial. The address is a one-time instruction: the room can be
 * unmounted and mounted again, and that must not dial the call a second time.
 */
let launchConsumed = false;

export function popOutCall(channelId: string, serverId: string): void {
  if (serverId.length === 0) return;
  const url = new URL(window.location.href);
  url.searchParams.set("popout", "1");
  url.searchParams.set("voiceChannel", channelId);
  // A named window can already exist on another server. Its session storage
  // is independent, so the launch address must carry the target explicitly.
  url.searchParams.set("serverId", serverId);
  window.open(url, "stellarr-call", "popup,width=1100,height=760");
}

/** Joins the call this window was opened for, the first time a room mounts. */
export function useJoinFromPopoutLink(join: (channelId: string) => void): void {
  useEffect(() => {
    if (launchConsumed) return;
    launchConsumed = true;
    const params = new URLSearchParams(window.location.search);
    const channelId = params.get("voiceChannel");
    if (params.get("popout") === "1" && channelId !== null) join(channelId);
  }, [join]);
}
