import { DmChat } from "@/components/friends/DmChat";
import { MediaViewer } from "@/components/room/attachments/MediaViewer";
import { useEmoji, useFriends, useProfile } from "@/state/hooks";

import { toPerson } from "./toPerson";
import { useAttachmentViewer } from "./useAttachmentViewer";
import { useChatTranslation } from "./useChatTranslation";
import { useDmEncryptionView } from "./useDmEncryption";

/**
 * The open conversation's messages and composer, wired to the friends state.
 * The room renders it outside a call and the chat beside the call during one,
 * never both at once.
 */
export function DmChatContainer() {
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
  if (conversation === null) return null;

  return (
    <>
      <DmChat
        peer={toPerson(conversation.peer)}
        username={conversation.peer.username}
        sections={friends.sections}
        onSend={friends.send}
        hasOlderMessages={friends.hasOlderMessages}
        loadingOlderMessages={friends.loadingOlderMessages}
        onLoadOlderMessages={friends.loadOlderMessages}
        chips={friends.chips}
        onAttach={friends.attach}
        onRemoveChip={friends.removeChip}
        onRetryChip={friends.retryChip}
        onReorderChips={friends.reorderChips}
        onOpenProfile={profile.toggle}
        customEmoji={customEmoji}
        translation={translation}
        attachments={viewer.attachments}
        {...encryptionView}
      />
      <MediaViewer target={viewer.target} onClose={viewer.close} />
    </>
  );
}
