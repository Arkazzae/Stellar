import { useMemo, useSyncExternalStore } from "react";

import { deleteScheduled, messageSubject } from "@/components/confirmations";
import { useConfirm } from "@/components/primitives/confirmContext";
import type { ScheduledListActions, ScheduledTargetLabel } from "@/components/scheduled/ScheduledList";
import { readerZone } from "@/lib/schedule/zonedTime";
import type { Translator } from "@/i18n";
import type { DmConversation, ScheduledMessage } from "@/ports";
import { useDirectory, useFriends, useLanguage, usePorts, useSession } from "@/state/hooks";

/** The author's scheduled messages, current as the live events arrive. */
export function useScheduledItems(): readonly ScheduledMessage[] {
  const { scheduled } = usePorts();
  return useSyncExternalStore(scheduled.subscribe, scheduled.getSnapshot, scheduled.getSnapshot);
}

/** The zone the reader's clock is in; read once, as the app reads it nowhere else. */
export function useReaderZone(): string {
  return useMemo(() => readerZone(), []);
}

/** A field a scheduled message can be written in: a channel, a thread of it, a conversation. */
export type ScheduledPlace =
  | { kind: "channel"; channelId: string; threadRootId?: string }
  | { kind: "conversation"; conversationId: string };

/**
 * Whether `item` goes to `place`. A thread has no place of its own on the
 * wire: a message for it answers the thread's first message. The channel's
 * own field counts everything going to the channel, its threads included.
 */
export function goesTo(item: ScheduledMessage, place: ScheduledPlace): boolean {
  if (place.kind === "conversation") {
    return item.target.kind === "conversation" && item.target.conversationId === place.conversationId;
  }
  if (item.target.kind !== "channel" || item.target.channelId !== place.channelId) return false;
  return place.threadRootId === undefined || item.replyToMessageId === place.threadRootId;
}

/** How many wait to go out to `place`, and when the first does: the bar over its field. */
export function useWaitingHere(place: ScheduledPlace | null): { count: number; next: Date | null } {
  const items = useScheduledItems();
  if (place === null) return { count: 0, next: null };
  // The list holds at most a few dozen; the snapshot is already soonest first.
  const here = items.filter((item) => item.state === "pending" && goesTo(item, place));
  const first = here[0];
  return { count: here.length, next: first === undefined ? null : new Date(first.sendAt) };
}

/** What the client knows of the places a message can go to. */
export interface KnownPlaces {
  /** The server on screen: its channels need no server's name beside them. */
  serverId: string;
  textChannels: readonly { id: string; name: string }[];
  conversations: readonly Pick<DmConversation, "id" | "peer">[];
}

/**
 * Where a scheduled message goes, as the list names it: "# general", a
 * channel elsewhere with its server's name, or the person a conversation is
 * with. The names the message carries are used where the client has none.
 */
export function scheduledTargetLabel(item: ScheduledMessage, places: KnownPlaces, t: Translator): ScheduledTargetLabel {
  const { target } = item;
  if (target.kind === "conversation") {
    const peer = places.conversations.find((conversation) => conversation.id === target.conversationId)?.peer;
    if (peer === undefined) return { text: t("scheduled.target.conversation") };
    return {
      text: t("scheduled.target.person", { name: peer.displayName }),
      ...(peer.avatarUrl === undefined ? {} : { avatarUrl: peer.avatarUrl }),
    };
  }
  const here = target.serverId === undefined || target.serverId === places.serverId;
  const channel = places.textChannels.find((candidate) => candidate.id === target.channelId)?.name ?? target.channelName;
  // Without a name the channel, or its whole server, is gone.
  if (channel === undefined) return { text: t("scheduled.target.goneChannel") };
  if (here || target.serverName === undefined) return { text: t("scheduled.target.channel", { channel }) };
  return { text: t("scheduled.target.channelOnServer", { channel, server: target.serverName }) };
}

/**
 * What the scheduled list needs beyond the messages: their places named, and
 * the actions. Deleting asks first on the question card; the other actions
 * act at once, as each can be undone or done again.
 */
export function useScheduledListActions(): ScheduledListActions & {
  targetOf(item: ScheduledMessage): ScheduledTargetLabel;
} {
  const { scheduled } = usePorts();
  const { server, textChannels } = useDirectory();
  const { conversations } = useFriends();
  const { user } = useSession();
  const { t } = useLanguage();
  const confirm = useConfirm();

  const targetOf = (item: ScheduledMessage) =>
    scheduledTargetLabel(item, { serverId: server.id, textChannels, conversations }, t);

  return {
    targetOf,
    onSendNow: (id) => scheduled.sendNow(id),
    onEdit: (id, content) => scheduled.edit(id, { content }),
    onReschedule: (id, at) => scheduled.edit(id, { sendAt: at }),
    onDelete: async (id) => {
      const item = scheduled.getSnapshot().find((candidate) => candidate.id === id);
      if (item === undefined) return;
      await confirm({
        ...deleteScheduled(
          t,
          messageSubject(t, {
            authorId: user?.id ?? "",
            authorName: user?.displayName ?? "",
            ...(user?.avatarUrl === undefined ? {} : { authorAvatarUrl: user.avatarUrl }),
            text: item.content,
          }),
        ),
        run: () => scheduled.remove(id),
      });
    },
    onCopy: (content) => navigator.clipboard.writeText(content),
  };
}
