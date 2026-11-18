import { useEffect, useMemo, useRef } from "react";

import { contiguousRun, createMessageStore } from "@/lib/messageStore/messageStore";
import type { Message } from "@/ports";

import type { ChannelWindow, ChatData } from "./chatStore";
import type { ChatDispatch, PendingIds, RestoredWindows, SyncCursors } from "./shared";

/** How long a change waits before it is written; a burst is written once. */
export const WRITE_DELAY_MS = 2_000;

interface MessageStoreOptions {
  /** Whose store it is; null while nobody is signed in, and nothing is kept then. */
  userId: string | null;
  serverId: string;
  /** The channel on screen; the one painted from the device when it opens. */
  channelId: string | null;
  /** Every text channel of the open server. */
  channelIds: readonly string[];
  data: ChatData;
  dispatch: ChatDispatch;
  pendingIds: PendingIds;
  syncCursors: SyncCursors;
  restored: RestoredWindows;
}

interface Written {
  messages: Message[];
  window: ChannelWindow;
}

/**
 * Keeps the newest messages of the channels that were read on the device, and
 * paints them when a channel opens before the server has answered.
 *
 * Reads and writes happen off the render path, and a write waits for changes to
 * settle: typing and scrolling never wait for the disk. Only a channel the
 * server has answered for is written, and always the messages together with the
 * cursor that belongs to them, taken from one snapshot of the state.
 */
export function useMessageStore({
  userId,
  serverId,
  channelId,
  channelIds,
  data,
  dispatch,
  pendingIds,
  syncCursors,
  restored,
}: MessageStoreOptions): void {
  const store = useMemo(
    () => (userId === null ? null : createMessageStore({ userId, serverId })),
    [userId, serverId],
  );
  const latest = useRef(data);
  const asked = useRef(new Set<string>());
  const written = useRef(new Map<string, Written>());
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  useEffect(() => {
    latest.current = data;
  });

  useEffect(() => {
    asked.current.clear();
    written.current.clear();
  }, [store]);

  // Channels that are gone, or no longer readable, take their stored window
  // with them as soon as the list says so.
  useEffect(() => {
    if (store !== null) void store.keepOnly(channelIds);
  }, [store, channelIds]);

  const loaded = channelId === null ? false : data.loaded[channelId] === true;
  useEffect(() => {
    if (store === null || channelId === null || loaded || asked.current.has(channelId)) return;
    asked.current.add(channelId);
    void store.read(channelId).then((stored) => {
      if (stored === undefined || stored.messages.length === 0) return;
      // The server was quicker, or failed: its answer is the newer word.
      if (latest.current.loaded[channelId] === true) return;
      if (syncCursors.current[channelId] !== undefined) return;
      const oldest = stored.messages[0];
      if (oldest === undefined) return;
      restored.current[channelId] = {
        ids: new Set(stored.messages.map((message) => message.id)),
        cursor: stored.cursor,
        floor: oldest,
      };
      dispatch({ type: "stored-window", channelId, messages: stored.messages });
    });
  }, [store, channelId, loaded, dispatch, syncCursors, restored]);

  const { messages, windows } = data;
  useEffect(() => {
    if (store === null || timer.current !== undefined) return;
    const dirty = (snapshot: ChatData) =>
      Object.keys(snapshot.windows).some((id) => needsWrite(snapshot, id, written.current));
    if (!dirty(latest.current)) return;
    timer.current = setTimeout(() => {
      timer.current = undefined;
      const snapshot = latest.current;
      for (const [id, window] of Object.entries(snapshot.windows)) {
        const bucket = snapshot.messages[id];
        if (bucket === undefined || !needsWrite(snapshot, id, written.current)) continue;
        written.current.set(id, { messages: bucket, window });
        void store.write(
          id,
          contiguousRun(bucket, window.floor),
          window.cursor,
          new Set(pendingIds.current),
        );
      }
    }, WRITE_DELAY_MS);
  }, [store, messages, windows, pendingIds]);

  useEffect(
    () => () => {
      clearTimeout(timer.current);
      timer.current = undefined;
    },
    [],
  );
}

/** A channel the server has answered for whose bucket or window changed since it was written. */
function needsWrite(data: ChatData, channelId: string, written: Map<string, Written>): boolean {
  const bucket = data.messages[channelId];
  const window = data.windows[channelId];
  if (bucket === undefined || window === undefined || data.loaded[channelId] !== true) {
    return false;
  }
  const before = written.get(channelId);
  return before?.messages !== bucket || before.window !== window;
}
