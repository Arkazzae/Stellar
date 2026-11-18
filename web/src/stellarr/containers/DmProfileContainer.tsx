import { DmProfilePanel } from "@/components/friends/DmProfilePanel";
import { formatDate } from "@/lib/format/format";
import { useFriends, useLanguage, useProfile, useSession, useUi } from "@/state/hooks";

import { toPerson } from "./FriendsRoomContainer";
import { relationTo } from "./relationTo";

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
  const { locale } = useLanguage();

  const conversation = friends.active;
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
    />
  );
}
