import { DmEncryptionRows } from "@/components/friends/DmEncryptionRows";
import { DmProfilePanel } from "@/components/friends/DmProfilePanel";
import { unsupportedReason } from "@/components/friends/encryptionCopy";
import { formatDate } from "@/lib/format/format";
import { useEncryption, useFriends, useLanguage, useProfile, useSession, useUi } from "@/state/hooks";

import { toPerson } from "./toPerson";
import { relationTo } from "./relationTo";
import { switchView, useSafetyStanding } from "./useDmEncryption";

/**
 * The profile of the person in the open conversation, in the roster's column.
 *
 * Everything on it is already in the client: the conversation names the
 * person, the friends graph says what you are to each other and since when.
 * Nothing is fetched for it.
 */
export function DmProfileContainer() {
  const ui = useUi();
  const friends = useFriends();
  const profile = useProfile();
  const { user } = useSession();
  const { locale, t } = useLanguage();
  const encryption = useEncryption();

  const conversation = friends.active;
  const view = conversation === null ? "off" : switchView(conversation, user?.id);
  const code = useSafetyStanding(conversation?.id ?? null, view === "on" && ui.profilePanelOpen);
  if (conversation === null) return null;

  const { peer } = conversation;
  const friend = friends.friends.find((entry) => entry.user.id === peer.id);
  const customStatus = peer.customStatus?.trim();

  return (
    <DmProfilePanel
      open={ui.profilePanelOpen}
      person={toPerson(peer)}
      username={peer.username}
      {...(customStatus ? { customStatus } : {})}
      {...(friend === undefined ? {} : { friendsSince: formatDate(friend.since, locale) })}
      relation={relationTo(peer.id, user?.id, friends)}
      onClose={() => ui.setProfilePanelOpen(false)}
      onOpenProfile={(anchor) => profile.toggle(peer.id, anchor)}
      onRemoveFriend={() => friends.remove(peer.id)}
      onBlock={() => friends.block(peer.id)}
      onUnblock={() => friends.unblock(peer.id)}
      encryption={
        <DmEncryptionRows
          peerName={peer.displayName}
          state={view}
          {...(conversation.encryption?.enabledAt
            ? { enabledOn: formatDate(conversation.encryption.enabledAt, locale) }
            : {})}
          unavailable={unsupportedReason(t, encryption.status.support)}
          busy={encryption.switching === conversation.id}
          error={encryption.switchError?.conversationId === conversation.id ? encryption.switchError.message : null}
          {...(code === undefined ? {} : { code })}
          onSwitch={(next) => (next ? encryption.switchOn(conversation) : encryption.switchOff(conversation))}
          onOpenCode={() => encryption.openDialog({ kind: "safety", conversationId: conversation.id })}
        />
      }
    />
  );
}
