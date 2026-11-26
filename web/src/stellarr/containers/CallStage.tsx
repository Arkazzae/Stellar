import { useMemo, useSyncExternalStore } from "react";

import type { ActivityMark } from "@/components/voice/ParticipantTiles";
import { VoiceControlBar, type BarActivity } from "@/components/voice/VoiceControlBar";
import { VoiceStage } from "@/components/voice/VoiceStage";
import { isFull } from "@/lib/activities/api";
import { TALK_ACTION } from "@/lib/keybinds/keybinds";
import {
  useActivities,
  useActivitySlot,
  useDirectory,
  useKeybinds,
  useLanguage,
  usePorts,
  useProfile,
  useSession,
  useUi,
  useVoice,
} from "@/state/hooks";

import { ActivityDevPreview } from "./ActivityDevPreview";
import { activityPanelOf } from "./ActivityHost";
import { popOutCall } from "./callPopout";
import { VoiceProblemNotice } from "./VoiceProblemNotice";

interface CallStageProps {
  /** The element full screen is asked of: the whole room, not just the stage. */
  fullscreenTarget: () => HTMLElement | null;
}

/**
 * The call's own view of the room: who is in it, what is shared, and the
 * controls. An activity you are inside takes the stage, drawn by the app's
 * activity host over the slot here, and the people line up under it.
 */
export function CallStage({ fullscreenTarget }: CallStageProps) {
  const voice = useVoice();
  const activities = useActivities();
  const voiceSlot = useActivitySlot("voice");
  const { t } = useLanguage();
  const ui = useUi();
  const profile = useProfile();
  const { user } = useSession();
  const { server, canManageServer } = useDirectory();
  const ports = usePorts();
  const { bindings } = useKeybinds();

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

  const live = joinedChannelId === null ? null : (activities.live.get(joinedChannelId) ?? null);
  const inside = activities.voice !== null && activities.voice.place === joinedChannelId;
  const mark = useMemo<ActivityMark | undefined>(
    () =>
      live === null
        ? undefined
        : { install: live.install, name: live.name, icon: live.icon, people: new Set(live.participants) },
    [live],
  );
  const { openPicker, join, joining, joinFailure, ended } = activities;
  let activity: BarActivity | undefined;
  if (joinedChannelId !== null && !inside) {
    activity =
      live === null
        ? { kind: "pick", onOpen: () => openPicker({ scope: "voice", channelId: joinedChannelId, change: false }) }
        : {
            kind: "join",
            install: live.install,
            name: live.name,
            icon: live.icon,
            busy: joining === joinedChannelId,
            full: isFull(live)
              ? t("activities.fullDetail", { count: live.participants.length, seats: live.seats ?? 0 })
              : null,
            error: joinFailure?.channelId === joinedChannelId ? joinFailure.text : null,
            onJoin: () => join(joinedChannelId),
          };
  }

  const participants = (
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
      underActivity={inside}
      activityMark={mark}
    />
  );

  return (
    <>
      <VoiceProblemNotice />
      {ended !== null && ended.place === joinedChannelId ? (
        <p className="act-ended" role="status">
          {t("activities.ended", { name: ended.name })}
        </p>
      ) : null}
      <div className="call-stage" data-activity={inside ? (ui.cinema ? "cinema" : "open") : undefined}>
        {inside ? <div className="act-slot" ref={voiceSlot} /> : null}
        {participants}
      </div>
      {import.meta.env.DEV && typeof __ACTIVITY_PREVIEW__ !== "undefined" &&
        __ACTIVITY_PREVIEW__ && joinedChannelId &&
        <ActivityDevPreview key={"preview-" + joinedChannelId} channel={joinedChannelId} muted={voice.state.muted} cameraOn={voice.call?.cameraOn} />}
      <VoiceControlBar
        muted={voice.state.muted}
        talkKey={bindings[TALK_ACTION]}
        talking={voice.state.talking}
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
        // Inside an activity, full screen is the activity's: the room
        // around it would leave its panel, which is drawn outside the room.
        fullscreenTarget={inside ? () => activityPanelOf("voice") : fullscreenTarget}
        onToggleMute={voice.toggleMute}
        onToggleDeafen={voice.toggleDeafen}
        onToggleCamera={voice.toggleCamera}
        onSelectMicrophone={openDeviceSettings}
        onSelectCamera={openDeviceSettings}
        activity={activity}
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
