import { useState } from "react";
import type { Message } from "@/ports";
import { LaunchTarget, type LaunchProps } from "./LaunchTarget";
import { ConfirmProvider } from "@/components/primitives/ConfirmDialog";
import { AppShell } from "@/components/shell/AppShell";
import { AccountProvider } from "@/state/AccountProvider";
import { ChatProvider } from "@/state/ChatProvider";
import { DirectoryProvider } from "@/state/DirectoryProvider";
import { EmojiProvider } from "@/state/EmojiProvider";
import { EncryptionProvider } from "@/state/EncryptionProvider";
import { FriendsProvider } from "@/state/FriendsProvider";
import { KeybindProvider } from "@/state/KeybindProvider";
import { NotificationProvider } from "@/state/NotificationProvider";
import { useFriends, useUi } from "@/state/hooks";
import { ProfileProvider } from "@/state/ProfileProvider";
import { UiProvider } from "@/state/UiProvider";
import { VoiceProvider } from "@/state/VoiceProvider";

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
                            <Shell {...props} />
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

function Shell({ entry, onEntryHandled }: LaunchProps) {
  const [noticeMessage, setNoticeMessage] = useState<Message | null>(null);
  const ui = useUi();
  const { active } = useFriends();
  const dm = ui.space === "dm";
  // The roster's column, beside a conversation, holds the other person's
  // profile, or during its call the chat; one at a time. The Friends view
  // lists people itself, so there it stays shut.
  const conversation = dm && active !== null;

  return (
    <>
      {/* The face and the wallpaper: what of the theme only the signed-in app
          shows. The wallpaper sits on a fixed layer behind the shell. */}
      <AppAppearance />
      {entry && onEntryHandled && <LaunchTarget key={entry} entry={entry} finish={onEntryHandled} reveal={setNoticeMessage} />}
      <AppShell
        serverRail={<ServerRailContainer />}
        // One column, two contents. Both rails are unmounted when the other is
        // showing rather than hidden: the DM rail holds a search field and the
        // channel rail a drag state, and neither should survive a switch.
        rail={dm ? <DmRailContainer /> : <RailContainer />}
        room={dm ? <FriendsRoomContainer /> : <RoomContainer noticeMessage={noticeMessage} onNoticeConsumed={setNoticeMessage} />}
        roster={
          dm ? (
            conversation ? (
              <>
                <DmProfileContainer />
                <DmCallChatContainer />
              </>
            ) : null
          ) : (
            <RosterContainer />
          )
        }
        dmSpace={dm}
        railOpen={ui.railOpen}
        onRailOpenChange={ui.setRailOpen}
        rosterOpen={
          dm ? conversation && (ui.profilePanelOpen || ui.callChatOpen) : ui.rosterOpen
        }
        onRosterOpenChange={
          dm ? (ui.callChatOpen ? ui.setCallChatOpen : ui.setProfilePanelOpen) : ui.setRosterOpen
        }
        stageFull={ui.cinema}
        onExitStageFull={() => ui.setCinema(false)}
      />
      {/* Outside the grid: the dialogs are fixed overlays, and mounting them
          inside `.app` would put them under the shell's stacking context. */}
      <ModalsContainer />
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
