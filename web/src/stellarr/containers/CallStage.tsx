import { useState, useSyncExternalStore } from "react";

import { VoiceControlBar } from "@/components/voice/VoiceControlBar";
import { VoiceStage } from "@/components/voice/VoiceStage";
import {
  useDirectory,
  usePorts,
  useProfile,
  useSession,
  useUi,
  useVoice,
} from "@/state/hooks";

import { ActivityDevPreview } from "./ActivityDevPreview";
import { InstalledActivities, type VoiceActivityBarControl } from "./InstalledActivities";
import { popOutCall } from "./callPopout";
import { VoiceProblemNotice } from "./VoiceProblemNotice";

interface CallStageProps {
  /** The element full screen is asked of: the whole room, not just the stage. */
  fullscreenTarget: () => HTMLElement | null;
}

/** The call's own view of the room: who is in it, what is shared, and the controls. */
export function CallStage({ fullscreenTarget }: CallStageProps) {
  const voice = useVoice();
  const [pickerOpen, setPickerOpen] = useState(false);
  const [barActivity, setBarActivity] = useState<VoiceActivityBarControl | null>(null);
  const ui = useUi();
  const profile = useProfile();
  const { user } = useSession();
  const { server, canManageServer } = useDirectory();
  const ports = usePorts();

  // The device settings straight from the store the pipelines read, so what
  // the bar's arrows edit is what the call is running on.
  const settings = useSyncExternalStore(
    ports.voiceSettings.subscribe,
    ports.voiceSettings.getSnapshot,
    ports.voiceSettings.getSnapshot,
  );

  // Device pickers live in the voice settings dialog. The server's own dialog
  // is the wrong place: the microphone arrow once raised roles and invites.
  const openDeviceSettings = () => ui.openModal("voice");
  const joinedChannelId = voice.channelId;
  const activeVoiceBar = joinedChannelId !== null ? barActivity : null;

  return (
    <>
      <VoiceProblemNotice />
      {joinedChannelId && (
        <InstalledActivities
          channel={joinedChannelId}
          pickerOpen={pickerOpen}
          onClosePicker={() => setPickerOpen(false)}
          onVoiceBarChange={setBarActivity}
          muted={voice.state.muted}
          cameraOn={voice.call?.cameraOn}
        />
      )}
      {import.meta.env.DEV && typeof __ACTIVITY_PREVIEW__ !== "undefined" &&
        __ACTIVITY_PREVIEW__ && joinedChannelId &&
        <ActivityDevPreview key={"preview-" + joinedChannelId} channel={joinedChannelId} muted={voice.state.muted} cameraOn={voice.call?.cameraOn} />}
      <VoiceStage
        people={voice.people}
        cameraStreams={voice.cameraStreams}
        heldCameras={voice.heldCameras}
        localPersonId={user?.id ?? ""}
        participantVolumes={voice.participantVolumes}
        onParticipantVolumeChange={voice.setParticipantVolume}
        participantGates={voice.participantGates}
        onParticipantGateChange={voice.setParticipantGate}
        participantSuppressions={voice.participantSuppressions}
        onParticipantSuppressionChange={voice.setParticipantSuppression}
        soundPlays={ports.soundboard.live}
        onOpenProfile={profile.toggle}
        shares={voice.shares}
        viewers={voice.viewers}
        onWatchShare={voice.watchShare}
        onStopWatchingShare={voice.stopWatchingShare}
        onStopSharing={voice.toggleShare}
        cinema={ui.cinema}
      />
      <VoiceControlBar
        muted={voice.state.muted}
        deafened={voice.call?.deafened ?? false}
        cameraOn={voice.call?.cameraOn ?? false}
        cameraPending={voice.state.cameraPending}
        sharing={voice.localShare}
        cinema={ui.cinema}
        sounds={voice.sounds}
        soundboardSupported={voice.soundboardEnabled && voice.soundboard.canPlay}
        soundboardLive={ports.soundboard.live}
        onManageSounds={voice.soundboard.canManage ? () => ui.openModal("soundboard") : undefined}
        settings={settings}
        onChangeSettings={(patch) => ports.voiceSettings.update(patch)}
        suppressorDownload={voice.suppressorDownload}
        cameraSupported={voice.cameraSupported}
        fullscreenTarget={fullscreenTarget}
        onToggleMute={voice.toggleMute}
        onToggleDeafen={voice.toggleDeafen}
        onToggleCamera={voice.toggleCamera}
        onSelectMicrophone={openDeviceSettings}
        onSelectCamera={openDeviceSettings}
        liveActivity={activeVoiceBar?.live ?? null}
        activityJoined={activeVoiceBar?.joined ?? false}
        activityBusy={activeVoiceBar?.busy ?? false}
        onOpenActivities={joinedChannelId ? () => setPickerOpen(true) : undefined}
        onJoinActivity={activeVoiceBar?.join}
        onLeaveActivity={activeVoiceBar?.leave}
        onToggleShare={voice.toggleShare}
        onPlaySound={voice.playSound}
        onInvite={canManageServer ? () => ui.openModal("invite") : undefined}
        onToggleCinema={ui.toggleCinema}
        onLeave={ui.leaveCall}
        onPopOut={() => {
          if (joinedChannelId !== null) popOutCall(joinedChannelId, server.id);
        }}
      />
    </>
  );
}
