import { memo, useSyncExternalStore } from "react";

import { AccountRow } from "@/components/account/AccountRow";
import { VoiceDock } from "@/components/voice/VoiceDock";
import type { PersonVM } from "@/components/view-models";
import { initials } from "@/lib/format/format";
import { TALK_ACTION } from "@/lib/keybinds/keybinds";
import {
  useAccount,
  useEmoji,
  useFriends,
  useKeybinds,
  usePorts,
  useSession,
  useUi,
  useVoiceCall,
} from "@/state/hooks";

/** Stands in until the boot payload lands; a rail renders one frame before it. */
const UNKNOWN_SELF: PersonVM = {
  id: "",
  displayName: "…",
  initials: "?",
  presence: "offline",
};

/**
 * The bottom of the rail, whichever rail is on screen.
 *
 * A call and the account row belong to the session, not to the server side of
 * the app: leaving a call has to stay reachable from a conversation, and the
 * two rails would drift apart the first time one of them was edited alone.
 */
export const RailVoiceDock = memo(function RailVoiceDock() {
  const ui = useUi();
  // The dock's own slice of the call: a ring that flips in it renders nothing here.
  const voice = useVoiceCall();
  const { ringing } = useFriends();
  const { bindings } = useKeybinds();
  const { voiceSettings, soundboard } = usePorts();
  const settings = useSyncExternalStore(
    voiceSettings.subscribe,
    voiceSettings.getSnapshot,
    voiceSettings.getSnapshot,
  );

  // The call's own view is on screen, with its bar: a channel's stage, or the
  // conversation whose call this is.
  const callViewOpen = (!ui.activityOpen && ui.space === "server" && ui.roomView === "voice") || ui.inConversationCall;

  return (
    <VoiceDock
      call={voice.call}
      others={voice.others}
      ringing={ringing}
      collapsed={callViewOpen}
      sounds={voice.sounds}
      soundboardSupported={voice.soundboardEnabled && voice.soundboard.canPlay}
      soundboardLive={soundboard.live}
      onManageSounds={voice.soundboard.canManage ? () => ui.openModal("soundboard") : undefined}
      settings={settings}
      // Named on the microphone in push to talk: with a text channel open,
      // the dock is the only microphone on screen.
      talkKey={bindings[TALK_ACTION]}
      onChangeSettings={(patch) => voiceSettings.update(patch)}
      suppressorDownload={voice.suppressorDownload}
      // Device pickers and the rest of the sound live in the voice settings,
      // as from the bar's arrow.
      onOpenSoundSettings={() => ui.openModal("voice")}
      cameraSupported={voice.cameraSupported}
      onToggleMute={voice.toggleMute}
      onToggleDeafen={voice.toggleDeafen}
      onToggleCamera={voice.toggleCamera}
      onToggleShare={voice.toggleShare}
      onLeave={ui.leaveCall}
      // A private call has no soundboard, so the dock offers none.
      onPlaySound={voice.privateCall ? undefined : voice.playSound}
      onReturnToChannel={ui.returnToChannel}
    />
  );
});

export const RailAccount = memo(function RailAccount() {
  const { user, presence, setPresence, logout } = useSession();
  const ui = useUi();
  const { emoji: customEmoji } = useEmoji();
  const { account, update } = useAccount();

  // The presence is the status you chose, not the one the socket implies: this
  // row is where you read back what you set.
  const self: PersonVM =
    user === null
      ? UNKNOWN_SELF
      : {
          id: user.id,
          displayName: user.displayName,
          initials: initials(user.displayName),
          presence,
          ...(user.avatarUrl === undefined ? {} : { avatarUrl: user.avatarUrl }),
        };

  return (
    <AccountRow
      user={self}
      // Held by the shell, so the composer's /status can open it too.
      open={ui.accountPanelOpen}
      onOpenChange={ui.setAccountPanelOpen}
      onOpenSettings={() => ui.openModal("account")}
      panel={{
        presence,
        customStatus: account.customStatus,
        customEmoji,
        onSetPresence: setPresence,
        onSaveCustomStatus: (customStatus) => {
          void update({ customStatus }).catch(() => undefined);
        },
        onLogout: () => {
          void logout();
        },
      }}
    />
  );
});
