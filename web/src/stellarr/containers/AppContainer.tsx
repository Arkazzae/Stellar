import { useState, type ReactNode } from "react";
import { PERSONAL } from "@/lib/activities/api";
import type { Message } from "@/ports";
import { AppBadge } from "./AppBadge";
import { ShareTarget } from "./ShareTarget";
import { LaunchTarget, type LaunchProps } from "./LaunchTarget";
import { ConfirmProvider } from "@/components/primitives/ConfirmDialog";
import { AppShell } from "@/components/shell/AppShell";
import { AccountProvider } from "@/state/AccountProvider";
import { ActivitiesProvider } from "@/state/ActivitiesProvider";
import { ChatProvider } from "@/state/ChatProvider";
import { DirectoryProvider } from "@/state/DirectoryProvider";
import { EmojiProvider } from "@/state/EmojiProvider";
import { EncryptionProvider } from "@/state/EncryptionProvider";
import { FriendsProvider } from "@/state/FriendsProvider";
import { KeybindProvider } from "@/state/KeybindProvider";
import { NotificationProvider } from "@/state/NotificationProvider";
import { useActivities, useActivitySlot, useFriends, useLanguage, useUi } from "@/state/hooks";
import { ProfileProvider } from "@/state/ProfileProvider";
import { UiProvider } from "@/state/UiProvider";
import { VoiceProvider } from "@/state/VoiceProvider";

import { ActivityHost } from "./ActivityHost";
import { ActivityPicker } from "./ActivityPicker";
import { AppAppearance } from "./AppAppearance";
import { ClientSettingsSync } from "./ClientSettingsSync";
import { DmCallChatContainer } from "./DmCallChatContainer";
import { DmProfileContainer } from "./DmProfileContainer";
import { DmRailContainer } from "./DmRailContainer";
import { EncryptionDialogsContainer } from "./EncryptionDialogsContainer";
import { FriendsRoomContainer } from "./FriendsRoomContainer";
import { IncomingCallContainer } from "./IncomingCallContainer";
import { MiniProfileContainer } from "./MiniProfileContainer";
import { ModalsContainer } from "./ModalsContainer";
import { RailContainer } from "./RailContainer";
import { RoomContainer } from "./RoomContainer";
import { RosterContainer } from "./RosterContainer";
import { ServerRailContainer } from "./ServerRailContainer";
import { UserSettingsContainer } from "./UserSettingsContainer";

/**
 * The provider order is a dependency order, not a preference:
 *
 *   Directory  → Voice    the call names its channel and lists its occupants
 *   Voice      → Ui       leaving a call is a room-level action
 *   Voice + Ui → Keybind  voice shortcuts invoke actions owned by both layers
 *   Ui         → Chat     the transcript follows the active text channel
 *   Ui         → Friends  opening a conversation is a navigation, and reading
 *                         it is what marks it read
 *   Voice + Ui → Activities  a channel's activity lives in its call, and
 *                         joining one from outside first joins the call
 *
 * `Profile` depends on none of them (it holds one open card and the element it
 * hangs off), so it sits innermost, next to the tree that raises it. `Account`
 * depends on nothing either, but every list showing your face is under it, so
 * it goes outermost after `Confirm`. `Emoji` belongs to the server, so it sits
 * under `Directory`, above everything that renders or picks a message.
 *
 * `Confirm` depends on nothing and is depended on from everywhere (a menu
 * item, a toolbar button, a settings row), so it goes outside all of them: its
 * dialog has to outlive whatever raised it.
 *
 * Session and Ports sit above all of it, in `StellarrApp` (`app.tsx`),
 * because /login needs them and none of this.
 */
export function AppContainer(props: LaunchProps = {}) {
  return (
    <ConfirmProvider>
      <AccountProvider>
        <DirectoryProvider>
          <NotificationProvider>
            <EmojiProvider>
              <VoiceProvider>
                <UiProvider>
                  <KeybindProvider>
                    <EncryptionProvider>
                      <FriendsProvider>
                        <ChatProvider>
                          <ProfileProvider>
                            <ActivitiesProvider>
                              <Shell {...props} />
                            </ActivitiesProvider>
                          </ProfileProvider>
                        </ChatProvider>
                      </FriendsProvider>
                    </EncryptionProvider>
                  </KeybindProvider>
                </UiProvider>
              </VoiceProvider>
            </EmojiProvider>
          </NotificationProvider>
        </DirectoryProvider>
      </AccountProvider>
    </ConfirmProvider>
  );
}

function PersonalActivityRoom({ title }: { title: string }) {
  const slot = useActivitySlot("personal");
  return (
    <main className="room activity-room" aria-label={title}>
      <div className="act-slot" ref={slot} />
    </main>
  );
}

function Shell({ entry, onEntryHandled }: LaunchProps) {
  const [noticeMessage, setNoticeMessage] = useState<Message | null>(null);
  const ui = useUi();
  const { active } = useFriends();
  const { personal, ended } = useActivities();
  const { t } = useLanguage();
  const foreground = ui.activityOpen ? personal : null;
  const dm = ui.space === "dm";
  let room: ReactNode;
  let roster: ReactNode = null;
  let rosterOpen = false;
  if (foreground !== null) {
    room = <PersonalActivityRoom title={foreground.title} />;
  } else if (dm) {
    room = <FriendsRoomContainer />;
    if (active !== null) {
      roster = <><DmProfileContainer /><DmCallChatContainer /></>;
      rosterOpen = ui.profilePanelOpen || ui.callChatOpen;
    }
  } else {
    room = <RoomContainer noticeMessage={noticeMessage} onNoticeConsumed={setNoticeMessage} />;
    roster = <RosterContainer />;
    rosterOpen = ui.rosterOpen;
  }

  return (
    <>
      <AppAppearance />
      <AppBadge />
      {entry && onEntryHandled && (entry.startsWith("?share=")
        ? <ShareTarget key={entry} id={entry.slice(7)} finish={onEntryHandled} />
        : <LaunchTarget key={entry} entry={entry} finish={onEntryHandled} reveal={setNoticeMessage} />)}
      <AppShell
        serverRail={<ServerRailContainer />}
        // One column, two contents. Both rails are unmounted when the other is
        // showing rather than hidden: the DM rail holds a search field and the
        // channel rail a drag state, and neither should survive a switch.
        rail={dm ? <DmRailContainer /> : <RailContainer />}
        room={room}
        roster={roster}
        dmSpace={dm || foreground !== null}
        railOpen={ui.railOpen}
        onRailOpenChange={ui.setRailOpen}
        rosterOpen={rosterOpen}
        onRosterOpenChange={
          dm ? (ui.callChatOpen ? ui.setCallChatOpen : ui.setProfilePanelOpen) : ui.setRosterOpen
        }
        stageFull={ui.cinema}
        onExitStageFull={() => ui.setCinema(false)}
      />
      {/* The running activities' frames: outside the rooms, so going
          elsewhere hides a frame instead of unloading it, and laid over the
          place the open view keeps for it. */}
      <ActivityHost />
      {ended?.place === PERSONAL && (
        <p className="act-personal-ended" role="status">{t("activities.ended", { name: ended.name })}</p>
      )}
      {/* Outside the grid: the dialogs are fixed overlays, and mounting them
          inside `.app` would put them under the shell's stacking context. */}
      <ModalsContainer />
      <ActivityPicker />
      {/* Above the shell and beside the dialogs: these are raised from inside
          any of them and have to outlive whatever raised them. */}
      <UserSettingsContainer />
      {/* Raised from the conversation, its profile and the settings alike. */}
      <EncryptionDialogsContainer />
      {/* Renders nothing; keeps the theme, language, shortcuts and voice
          settings on the account. */}
      <ClientSettingsSync />
      <MiniProfileContainer />
      <IncomingCallContainer />
    </>
  );
}
