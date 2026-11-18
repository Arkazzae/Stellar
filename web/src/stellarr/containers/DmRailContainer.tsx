import { useEffect, useMemo, useState } from "react";

import { lineText } from "@/components/friends/encryptionCopy";
import { DmRail } from "@/components/shell/rail/DmRail";
import type { DmConversationVM } from "@/components/view-models";
import { useMediaQuery } from "@/hooks/useMediaQuery";
import { formatShortAgo, initials } from "@/lib/format/format";
import type { DmConversation } from "@/ports";
import { useFriends, useLanguage, useSession, useUi } from "@/state/hooks";

import { RailAccount, RailVoiceDock } from "./RailFooter";

type Translate = ReturnType<typeof useLanguage>["t"];

const inCall = (conversation: DmConversation) =>
  (conversation.callParticipants?.length ?? 0) > 0;

/** "2 min" has to become "3 min" without anything else happening. */
const CLOCK_TICK_MS = 60_000;

function useMinuteClock(): Date {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(new Date()), CLOCK_TICK_MS);
    return () => window.clearInterval(timer);
  }, []);
  return now;
}

/**
 * What the rail's second line says about a conversation.
 *
 * Composed here rather than in the rail, because the answer depends on who is
 * speaking and on whether anything has been said at all.
 */
function previewOf(
  t: Translate,
  conversation: DmConversation,
  selfId: string,
): string {
  const { lastMessage } = conversation;
  if (!inCall(conversation) && lastMessage?.line !== undefined) {
    // A line says what happened in its own words, already about the reader.
    const { line } = lastMessage;
    return lineText(t, line, lastMessage.author.displayName, {
      self: line.actorId === selfId,
      live: line.kind === "encryption_requested" && conversation.encryption?.state === "requested",
      peerName: conversation.peer.displayName,
    });
  }
  const content = inCall(conversation)
    ? t("dm.callPreview")
    : lastMessage?.sealed !== undefined
      ? t("e2ee.preview.sealed")
      : lastMessage?.content;
  if (content === undefined) return t("dm.firstMessage");
  // The rail is one line; a pasted paragraph must not decide its height.
  const flat = content.replace(/\s+/g, " ").trim();
  if (flat.length === 0) return t("dm.attachment");
  return lastMessage?.author.id === selfId
    ? t("dm.youPrefix", { content: flat })
    : flat;
}

export function DmRailContainer() {
  const ui = useUi();
  const friends = useFriends();
  const { t, locale } = useLanguage();
  const { user } = useSession();
  const selfId = user?.id ?? "";
  const now = useMinuteClock();

  // Below 768 the rail is a drawer; shut, it is off screen and must be inert.
  const railIsDrawer = useMediaQuery("(max-width: 767px)", false);

  const conversations = useMemo<DmConversationVM[]>(
    () =>
      friends.conversations.map((conversation) => {
        const { peer, lastMessage } = conversation;
        const sentAt = lastMessage?.createdAt;
        return {
          id: conversation.id,
          peer: {
            id: peer.id,
            displayName: peer.displayName,
            initials: initials(peer.displayName),
            presence: peer.presence,
            // The face, not just the letters: the rail is the one list where a
            // conversation is recognised before its name is read.
            ...(peer.avatarUrl === undefined ? {} : { avatarUrl: peer.avatarUrl }),
          },
          preview: previewOf(t, conversation, selfId),
          unread: conversation.unread,
          ...(inCall(conversation) ? { callActive: true } : {}),
          ...(conversation.encryption?.state === "on" ? { encrypted: true } : {}),
          ...(sentAt === undefined
            ? {}
            : { time: { text: formatShortAgo(sentAt, now, locale), iso: sentAt } }),
        };
      }),
    [friends.conversations, selfId, t, now, locale],
  );

  return (
    <DmRail
      friendsActive={ui.activeDmId === null}
      onOpenFriends={ui.openFriends}
      pending={friends.incoming}
      conversations={conversations}
      activeDmId={ui.activeDmId}
      onSelect={ui.openDm}
      onNewConversation={ui.newConversation}
      voiceDock={<RailVoiceDock />}
      account={<RailAccount />}
      hidden={railIsDrawer && !ui.railOpen}
    />
  );
}
