import { useState } from "react";

import { draftScope } from "@/lib/platform/drafts";
import { DmChat } from "@/components/friends/DmChat";
import { MediaViewer } from "@/components/room/attachments/MediaViewer";
import { useEmoji, useFriends, useProfile, useSession } from "@/state/hooks";

import { toPerson } from "./toPerson";
import { useAttachmentViewer } from "./useAttachmentViewer";
import { useChatTranslation } from "./useChatTranslation";
import { useDmEncryptionView } from "./useDmEncryption";
import { ScheduledDialogContainer } from "./ScheduledDialogContainer";
import { useReaderZone, useWaitingHere } from "./useScheduled";

/**
 * The open conversation's messages and composer, wired to the friends state.
 * The room renders it outside a call and the chat beside the call during one,
 * never both at once.
 */
export function DmChatContainer() {
  const { user } = useSession();
  const friends = useFriends();
  const profile = useProfile();
  const { emoji: customEmoji } = useEmoji();
  const translation = useChatTranslation(friends.active?.id);
  // A conversation carries the same pictures, clips and recordings as a text
  // channel, so it opens them in the same viewer.
  const viewer = useAttachmentViewer(friends.openAttachment);
  // Lines about encryption, sealed history and a composer that says why it is off.
  const encryptionView = useDmEncryptionView(friends.active);

  const conversation = friends.active;
  // The server takes only what is sealed for the key in use now, so an
  // encrypted conversation cannot hold a message for later.
  const place =
    conversation === null || conversation.encryption?.state === "on"
      ? null
      : { kind: "conversation" as const, conversationId: conversation.id };
  const zone = useReaderZone();
  const waiting = useWaitingHere(place);
  const [listOpen, setListOpen] = useState(false);
  const [listNow, setListNow] = useState(() => new Date());
  if (conversation === null) return null;

  return (
    <>
      <DmChat
        draftScope={user ? draftScope(user.id, "dm", conversation.id) : undefined}
        peer={toPerson(conversation.peer)}
        username={conversation.peer.username}
        sections={friends.sections}
        onSend={friends.send}
        hasOlderMessages={friends.hasOlderMessages}
        loadingOlderMessages={friends.loadingOlderMessages}
        onLoadOlderMessages={friends.loadOlderMessages}
        restoringFiles={friends.restoringFiles}
        chips={friends.chips}
        onAttach={friends.attach}
        onRemoveChip={friends.removeChip}
        onRetryChip={friends.retryChip}
        onReorderChips={friends.reorderChips}
        onOpenProfile={profile.toggle}
        customEmoji={customEmoji}
        translation={translation}
        attachments={viewer.attachments}
        {...(place === null
          ? {}
          : {
              scheduling: {
                onSchedule: friends.schedule,
                waiting,
                place: "conversation" as const,
                onShowScheduled: () => {
                  setListNow(new Date());
                  setListOpen(true);
                },
                zone,
              },
            })}
        {...encryptionView}
      />
      {listOpen && place !== null ? (
        <ScheduledDialogContainer place={place} now={listNow} zone={zone} onClose={() => setListOpen(false)} />
      ) : null}
      <MediaViewer target={viewer.target} onClose={viewer.close} resolveOriginal={viewer.attachments.resolveOriginal} />
    </>
  );
}
