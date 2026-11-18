import { useMemo, type ReactNode } from "react";

import { Icon } from "@stellarr/icons";
import { Avatar } from "@/components/primitives/Avatar";
import { Composer } from "@/components/room/composer/Composer";
import type { ComposerChip } from "@/components/room/composer/ComposerChips";
import type { MessageTranslationOptions } from "@/components/room/attachments/TranslatedTextAttachment";
import type { AttachmentCallbacks } from "@/lib/messages/parts";
import { Transcript, type TranscriptSectionVM } from "@/components/room/transcript/Transcript";
import { ParticipantTiles } from "@/components/voice/ParticipantTiles";
import type { OpenProfile, PersonVM } from "@/components/view-models";
import { translateForDocument } from "@/i18n/defaultLocale";
import { PRESENCE_LABEL, type CustomEmoji } from "@/ports";

export interface DmRoomProps {
  peer: PersonVM;
  /** Without the "@"; the intro prints it under the name. */
  username: string;
  /** What the person says about themselves, beside the presence word in the header. */
  status?: string;
  sections: TranscriptSectionVM[];
  railOpen: boolean;
  onToggleRail: () => void;
  /** The profile beside the conversation, and the header controls that show it. */
  profilePanelOpen: boolean;
  onToggleProfilePanel: () => void;
  onSend: (text: string) => Promise<void>;
  hasOlderMessages?: boolean;
  loadingOlderMessages?: boolean;
  onLoadOlderMessages?: () => void;
  /** Somebody is in the conversation's call. */
  callActive: boolean;
  /** You are in it. */
  inCall: boolean;
  onCall: () => void;
  videoOn: boolean;
  onVideoCall: () => void;
  callPeople: PersonVM[];
  cameraStreams: ReadonlyMap<string, MediaStream>;
  /** People whose camera is held back for the viewer's link. */
  heldCameras?: ReadonlySet<string>;
  localPersonId: string;
  participantVolumes: ReadonlyMap<string, number>;
  onParticipantVolumeChange: (personId: string, volume: number) => void;
  participantGates?: ReadonlyMap<string, boolean>;
  onParticipantGateChange?: (personId: string, gated: boolean) => void;
  participantSuppressions?: ReadonlyMap<string, boolean>;
  onParticipantSuppressionChange?: (personId: string, suppressed: boolean) => void;
  chips: ComposerChip[];
  onAttach: (files: File[]) => void;
  onRemoveChip: (chipId: string) => void;
  onRetryChip: (chipId: string) => void;
  onReorderChips: (from: number, to: number) => void;
  /** From an author in the transcript and from a tile. */
  onOpenProfile: OpenProfile;
  /** The server's own emoji: the picker and the reaction bar both offer them. */
  customEmoji?: readonly CustomEmoji[];
  translation?: MessageTranslationOptions;
  /** The same picture, clip and recording actions the channel transcript uses. */
  attachments?: AttachmentCallbacks;
  notificationButton?: ReactNode;
  /** One action under the intro's sentence. */
  introAction?: ReactNode;
}

/**
 * One conversation.
 *
 * The same room as a channel (header, transcript, composer), with three
 * differences that are the point of the surface: the title is a person, so it
 * wears their face and its topic says how reachable they are; the header
 * offers a call to that person and their profile instead of tools for a
 * channel; and the history has a beginning, stated once above the first
 * message rather than assumed.
 */
export function DmRoom({
  peer,
  username,
  status,
  sections,
  railOpen,
  onToggleRail,
  profilePanelOpen,
  onToggleProfilePanel,
  onSend,
  hasOlderMessages = false,
  loadingOlderMessages = false,
  onLoadOlderMessages,
  callActive,
  inCall,
  onCall,
  videoOn,
  onVideoCall,
  callPeople,
  cameraStreams,
  heldCameras,
  localPersonId,
  participantVolumes,
  onParticipantVolumeChange,
  participantGates,
  onParticipantGateChange,
  participantSuppressions,
  onParticipantSuppressionChange,
  chips,
  onAttach,
  onRemoveChip,
  onRetryChip,
  onReorderChips,
  onOpenProfile,
  customEmoji,
  translation,
  attachments,
  notificationButton,
  introAction,
}: DmRoomProps) {
  const t = translation?.t ?? translateForDocument;
  const name = peer.displayName;
  const presence = t(PRESENCE_LABEL[peer.presence]);
  const topic = status ? `${presence} · ${status}` : presence;

  const intro = useMemo(
    () => (
      <div className="dm-intro">
        <Avatar
          id={peer.id}
          displayName={name}
          initials={peer.initials}
          avatarUrl={peer.avatarUrl}
          size="xl"
        />
        <h3>{name}</h3>
        <span className="dm-handle">@{username}</span>
        <p>{t("dm.intro")}</p>
        {introAction === undefined ? null : <div className="dm-intro-act">{introAction}</div>}
      </div>
    ),
    [peer.id, peer.initials, peer.avatarUrl, name, username, introAction, t],
  );

  const callTitle = inCall ? t("dm.inCall") : t(callActive ? "dm.joinCall" : "dm.startCall");
  const callLabel = inCall
    ? t("dm.connected", { name })
    : t(callActive ? "dm.join" : "dm.call", { name });
  const cameraLabel = t(videoOn ? "voiceBar.cameraStop" : "voiceBar.cameraStart");
  // A camera held back for the viewer's link still shows its tile, which says so.
  const showTiles = inCall && (videoOn || cameraStreams.size > 0 || (heldCameras?.size ?? 0) > 0);

  return (
    <main className="room dm-room">
      <header className="room-header">
        <button
          type="button"
          className="burger"
          aria-label={t("dm.showConversations")}
          aria-expanded={railOpen}
          onClick={onToggleRail}
        >
          <Icon name="list" size={18} />
        </button>
        {/* The face and the name raise the profile as well: on a phone they
            are the way to it, because the panel's own button does not fit. */}
        <h2 className="dm-title">
          <button
            type="button"
            className="dm-who"
            aria-expanded={profilePanelOpen}
            onClick={onToggleProfilePanel}
          >
            <Avatar
              id={peer.id}
              displayName={name}
              initials={peer.initials}
              avatarUrl={peer.avatarUrl}
              size="xs"
              presence={peer.presence}
              title={presence}
            />
            <span className="dm-who-name">{name}</span>
          </button>
        </h2>
        <span className="topic">{topic}</span>

        <div className="pill-bar auto" role="toolbar" aria-label={t("dm.toolbar")}>
          <button
            type="button"
            className={inCall ? "pill-btn on" : "pill-btn"}
            title={callTitle}
            aria-label={callLabel}
            aria-pressed={inCall}
            onClick={onCall}
          >
            <Icon name="phone" size={16} />
          </button>
          <button
            type="button"
            className={videoOn ? "pill-btn on" : "pill-btn"}
            title={cameraLabel}
            aria-label={cameraLabel}
            aria-pressed={videoOn}
            onClick={onVideoCall}
          >
            <Icon name="camera" size={16} />
          </button>
          {notificationButton}
          <button
            type="button"
            className="pill-btn dm-profile-toggle"
            title={t("common.profile")}
            aria-label={t("common.profile")}
            aria-pressed={profilePanelOpen}
            onClick={onToggleProfilePanel}
          >
            <Icon name="channelPanel" size={16} />
          </button>
        </div>
      </header>

      <div className="room-body">
        <div className="view active">
          {showTiles ? (
            <div className="dm-video-stage">
              <ParticipantTiles
                people={callPeople}
                cameraStreams={cameraStreams}
                heldCameras={heldCameras}
                localPersonId={localPersonId}
                participantVolumes={participantVolumes}
                onParticipantVolumeChange={onParticipantVolumeChange}
                participantGates={participantGates}
                onParticipantGateChange={onParticipantGateChange}
                participantSuppressions={participantSuppressions}
                onParticipantSuppressionChange={onParticipantSuppressionChange}
                onOpenProfile={onOpenProfile}
              />
            </div>
          ) : null}
          <Transcript
            sections={sections}
            label={t("dm.messagesWith", { name })}
            onOpenAuthor={onOpenProfile}
            customEmoji={customEmoji}
            translation={translation}
            attachments={attachments}
            intro={intro}
            hasOlder={hasOlderMessages}
            loadingOlder={loadingOlderMessages}
            onLoadOlder={onLoadOlderMessages}
          />
          <Composer
            channelName={name}
            placeholder={t("dm.writeTo", { name })}
            chips={chips}
            onSubmit={onSend}
            onAttach={onAttach}
            onRemoveChip={onRemoveChip}
            onRetryChip={onRetryChip}
            onReorderChips={onReorderChips}
            customEmoji={customEmoji}
            locale={translation?.locale}
            t={t}
          />
        </div>
      </div>
    </main>
  );
}
