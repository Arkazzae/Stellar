import { useEffect } from "react";
import { getJson } from "@/lib/api/calls/http";
import { expectRecord, expectString } from "@/lib/api/expect";
import { parseMessage } from "@/lib/api/parsers";
import type { Message } from "@/ports";
import { useDirectory, useSession, useUi } from "@/state/hooks";

export interface LaunchProps {
  entry?: string | null;
  onEntryHandled?: () => void;
}

type Destination = { kind: "dm"; id: string } | { kind: "channel"; server: string; message: Message };

function destination(value: unknown, path: string): Destination {
  const row = expectRecord(value, path);
  if (row.kind === "dm") return { kind: "dm", id: expectString(row, "conversationId", path) };
  return { kind: "channel", server: expectString(row, "serverId", path), message: parseMessage(row.message, path) };
}

/** Keep the opaque URL until bootstrap has switched to the destination server. */
export function LaunchTarget({ entry, finish, reveal }: {
  entry: string; finish(): void; reveal(message: Message): void;
}) {
  const { server, selectServer } = useDirectory();
  const { reload } = useSession();
  const { openDm, openFriends, openModal, selectTextChannel } = useUi();
  useEffect(() => {
    const params = new URLSearchParams(entry);
    const id = params.get("notice");
    const abort = new AbortController();
    if (id === null) {
      if (params.get("view") === "messages") openFriends();
      else openModal("inbox");
      finish();
    } else {
      getJson("/api/me/notification-delivery/" + encodeURIComponent(id), destination, abort.signal).then(
        (target) => {
          if (abort.signal.aborted) return;
          if (target.kind === "dm") openDm(target.id);
          else if (server.id !== target.server) {
            selectServer(target.server);
            reload();
            return;
          } else {
            selectTextChannel(target.message.channelId);
            reveal(target.message);
          }
          finish();
        },
        () => {
          if (abort.signal.aborted) return;
          openModal("inbox");
          finish();
        },
      );
    }
    return () => abort.abort();
  }, [entry, finish, openDm, openFriends, openModal, reload, reveal, selectServer, selectTextChannel, server.id]);
  return null;
}
