import { useCallback, useMemo, useRef } from "react";

import { DmRoom } from "@/components/friends/DmRoom";
import { FriendsView } from "@/components/friends/FriendsView";
import type { FriendRequestVM, FriendVM } from "@/components/view-models";
import { formatAgo, formatDate } from "@/lib/format/format";
import { useFriends, useLanguage, useProfile, useUi, useVoice } from "@/state/hooks";

import { DmCallContainer } from "./DmCallContainer";
import { DmChatContainer } from "./DmChatContainer";
import { FriendsActivityContainer } from "./FriendsActivityContainer";
import { NotificationInboxButton } from "./NotificationInboxContainer";
import { toPerson } from "./toPerson";

/**
 * How long a video call waits before switching the camera on. The camera can
 * only be toggled once the call exists, and the join starts just before it.
 */
const CAMERA_AFTER_JOIN_MS = 500;

/**
 * What the room shows on the messages side of the app: one conversation, or
 * the list of people, never both. The rail already says who else is there, and
 * a second copy of it beside an open conversation would be a fourth column.
 */
export function FriendsRoomContainer() {
  const ui = useUi();
  const friends = useFriends();
  const profile = useProfile();
  const voice = useVoice();
  const { locale } = useLanguage();
  // Full screen takes the whole room, as on a channel: the bar's narrow layout
  // follows the room's width, which then is the screen's.
  const room = useRef<HTMLElement>(null);
  const fullscreenTarget = useCallback(() => room.current, []);

  const friendList = useMemo<FriendVM[]>(
    () =>
      friends.friends
        .map((friend) => {
          const customStatus = friend.user.customStatus?.trim();
          return {
            person: toPerson(friend.user),
            username: friend.user.username,
            since: formatDate(friend.since, locale),
            ...(customStatus ? { customStatus } : {}),
          };
        })
        .sort((a, b) => a.person.displayName.localeCompare(b.person.displayName, locale)),
    [friends.friends, locale],
  );

  const requests = useMemo<FriendRequestVM[]>(
    () =>
      friends.requests.map((request) => ({
        id: request.id,
        person: toPerson(request.user),
        username: request.user.username,
        direction: request.direction,
        when: formatAgo(request.sentAt, new Date(), locale),
      })),
    [friends.requests, locale],
  );

  const blocked = useMemo<FriendVM[]>(
    () => friends.blocked.map((person) => ({ person: toPerson(person), username: person.username })),
    [friends.blocked],
  );

  const conversation = friends.active;
  if (conversation !== null) {
    const inCall = voice.channelId === conversation.id;
    const status = conversation.peer.customStatus?.trim();
    return (
      <DmRoom
        ref={room}
        peer={toPerson(conversation.peer)}
        {...(status ? { status } : {})}
        railOpen={ui.railOpen}
        onToggleRail={ui.toggleRail}
        profilePanelOpen={ui.profilePanelOpen}
        onToggleProfilePanel={ui.toggleProfilePanel}
        callActive={(conversation.callParticipants?.length ?? 0) > 0}
        inCall={inCall}
        onCall={() => friends.callConversation(conversation.id)}
        videoOn={voice.call?.cameraOn ?? false}
        onVideoCall={() => {
          if (inCall) {
            voice.toggleCamera();
            return;
          }
          friends.callConversation(conversation.id);
          window.setTimeout(voice.toggleCamera, CAMERA_AFTER_JOIN_MS);
        }}
        chatOpen={ui.callChatOpen}
        onToggleChat={ui.toggleCallChat}
        chatUnread={conversation.unread}
        notificationButton={<NotificationInboxButton />}
        encrypted={conversation.encryption?.state === "on"}
      >
        {/* The call has the room while you are in it; its messages move
            into the column beside it. */}
        {inCall ? <DmCallContainer fullscreenTarget={fullscreenTarget} /> : <DmChatContainer />}
      </DmRoom>
    );
  }

  return (
    <FriendsView
      tab={ui.friendsTab}
      onTabChange={ui.setFriendsTab}
      searchRequested={ui.friendsSearchRequested}
      onSearchFocused={ui.takeFriendsSearch}
      friends={friendList}
      requests={requests}
      blocked={blocked}
      railOpen={ui.railOpen}
      onToggleRail={ui.toggleRail}
      onMessage={friends.messagePerson}
      onCall={friends.callPerson}
      onRemove={friends.remove}
      onBlock={friends.block}
      onUnblock={friends.unblock}
      onAccept={friends.accept}
      onDecline={friends.decline}
      onCancel={friends.cancel}
      onAddFriend={friends.addFriend}
      onOpenProfile={profile.toggle}
      notificationButton={<NotificationInboxButton />}
      activityPanel={<FriendsActivityContainer />}
    />
  );
}
