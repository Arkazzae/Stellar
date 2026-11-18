import type { ReactNode, Ref } from "react";

import { Icon } from "@stellarr/icons";
import { Avatar } from "@/components/primitives/Avatar";
import type { PersonVM } from "@/components/view-models";
import { PRESENCE_LABEL } from "@/ports";
import { useLanguage } from "@/state/LanguageProvider";

/** The most a pill spells out, as on a channel. */
const MAX_UNREAD_SHOWN = 99;

export interface DmRoomProps {
  peer: PersonVM;
  /** What the person says about themselves, beside the presence word in the header. */
  status?: string;
  railOpen: boolean;
  onToggleRail: () => void;
  /** The profile beside the conversation, and the header controls that show it. */
  profilePanelOpen: boolean;
  onToggleProfilePanel: () => void;
  /** Somebody is in the conversation's call. */
  callActive: boolean;
  /** You are in it. */
  inCall: boolean;
  onCall: () => void;
  videoOn: boolean;
  onVideoCall: () => void;
  /** The messages beside the call, which the header offers while you are in it. */
  chatOpen: boolean;
  onToggleChat: () => void;
  /** Messages that arrived while the chat was shut; its button counts them. */
  chatUnread?: number;
  notificationButton?: ReactNode;
  /** End-to-end encryption is on: a lock beside the name. */
  encrypted?: boolean;
  /**
   * Why this browser cannot call in this conversation, which is encrypted:
   * the call buttons stay, unusable, and say so.
   */
  callBlocked?: string;
  /** Under the header: the conversation, or its call while you are in it. */
  children: ReactNode;
  /** The room, which is what a call asks to put into full screen. */
  ref?: Ref<HTMLElement>;
}

/**
 * One conversation.
 *
 * The same room as a channel, with two differences that are the point of the
 * surface: the title is a person, so it wears their face and its topic says
 * how reachable they are; and the header offers a call to that person and
 * their profile instead of tools for a channel. During the call the room is
 * the call's, and the header's last button brings the messages beside it in
 * place of the profile, which the face and name still open.
 */
export function DmRoom({
  peer,
  status,
  railOpen,
  onToggleRail,
  profilePanelOpen,
  onToggleProfilePanel,
  callActive,
  inCall,
  onCall,
  videoOn,
  onVideoCall,
  chatOpen,
  onToggleChat,
  chatUnread = 0,
  notificationButton,
  encrypted = false,
  callBlocked,
  children,
  ref,
}: DmRoomProps) {
  const { t } = useLanguage();
  const name = peer.displayName;
  const presence = t(PRESENCE_LABEL[peer.presence]);
  const topic = status ? `${presence} · ${status}` : presence;

  const callTitle = inCall ? t("dm.inCall") : t(callActive ? "dm.joinCall" : "dm.startCall");
  const callLabel = inCall
    ? t("dm.connected", { name })
    : t(callActive ? "dm.join" : "dm.call", { name });
  const cameraLabel = t(videoOn ? "voiceBar.cameraStop" : "voiceBar.cameraStart");
  // A blocked call keeps its buttons in reach, so the reason can be read
  // from them, and does nothing when pressed.
  const blocked = callBlocked !== undefined && !inCall;
  const blockedProps = blocked ? { "aria-disabled": true, "aria-description": callBlocked } : {};

  return (
    <main className={inCall ? "room dm-room in-call" : "room dm-room"} ref={ref}>
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
            are the way to it, because the panel's own button does not fit,
            and during a call the chat takes that button. */}
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
            {encrypted ? (
              <span className="dm-lock" title={t("e2ee.lock")}>
                <Icon name="lock" size={14} />
                <span className="sr-only">{t("e2ee.lock")}</span>
              </span>
            ) : null}
          </button>
        </h2>
        <span className="topic">{topic}</span>

        <div className="pill-bar auto" role="toolbar" aria-label={t("dm.toolbar")}>
          <button
            type="button"
            className={inCall ? "pill-btn on" : "pill-btn"}
            title={blocked ? callBlocked : callTitle}
            aria-label={callLabel}
            aria-pressed={inCall}
            {...blockedProps}
            onClick={blocked ? undefined : onCall}
          >
            <Icon name="phone" size={16} />
          </button>
          <button
            type="button"
            className={videoOn ? "pill-btn on" : "pill-btn"}
            title={blocked ? callBlocked : cameraLabel}
            aria-label={cameraLabel}
            aria-pressed={videoOn}
            {...blockedProps}
            onClick={blocked ? undefined : onVideoCall}
          >
            <Icon name="camera" size={16} />
          </button>
          {notificationButton}
          {inCall ? (
            <button
              type="button"
              className="pill-btn dm-chat-toggle"
              title={t("dm.chat")}
              aria-label={
                chatUnread > 0 ? t("dm.chatUnread", { count: chatUnread }) : t("dm.chat")
              }
              aria-pressed={chatOpen}
              onClick={onToggleChat}
            >
              <Icon name="thread" size={16} />
              {chatUnread > 0 ? (
                <span className="ch-unread" aria-hidden="true">
                  {chatUnread > MAX_UNREAD_SHOWN ? `${MAX_UNREAD_SHOWN}+` : chatUnread}
                </span>
              ) : null}
            </button>
          ) : (
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
          )}
        </div>
      </header>

      <div className="room-body">
        <div className="view active">{children}</div>
      </div>
    </main>
  );
}
