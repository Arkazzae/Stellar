import { useId, useState, type ReactNode } from "react";

import { Icon } from "@stellarr/icons";
import { ActivityIcon } from "@/components/activities/ActivityIcon";
import { Avatar } from "@/components/primitives/Avatar";
import type { DmConversationVM } from "@/components/view-models";
import { PRESENCE_LABEL } from "@/ports";
import { useLanguage } from "@/state/hooks";

/** The most a pill spells out, as on a channel. */
const MAX_COUNT_SHOWN = 99;

const counted = (count: number) =>
  count > MAX_COUNT_SHOWN ? `${MAX_COUNT_SHOWN}+` : String(count);

export interface DmRailProps {
  /** True while the Friends view is showing rather than a conversation. */
  friendsActive: boolean;
  onOpenFriends: () => void;
  /** Incoming friend requests waiting for an answer; the count on the Friends row. */
  pending: number;
  /** Opens the activities you can run for yourself. Absent hides the row. */
  onOpenActivities?: () => void;
  /** The activity you run for yourself, as its row under Activities shows it. */
  personalActivity?: PersonalActivityVM | null;
  onShowActivity?: () => void;
  onCloseActivity?: () => void;
  /** Said for a moment where your activity's row was, when it ended under you. */
  activityNotice?: string | null;
  conversations: DmConversationVM[];
  activeDmId: string | null;
  onSelect: (id: string) => void;
  /** Opens the Friends view on everyone, with its search ready for a name. */
  onNewConversation: () => void;
  /** `<VoiceDock>`, composed by the container. */
  voiceDock?: ReactNode;
  /** `<AccountRow>`, composed by the container. */
  account?: ReactNode;
  /**
   * Below 768 the rail is an off-screen drawer while shut. Off screen must
   * also mean out of the tab order and off the accessibility tree, the same
   * way the roster handles it.
   */
  hidden?: boolean;
}

/** The activity you run for yourself, in the rail. */
export interface PersonalActivityVM {
  install: string;
  name: string;
  icon: string | null;
  /** It has the room right now, rather than running in the background. */
  shown: boolean;
}

interface ConversationRowProps {
  conversation: DmConversationVM;
  current: boolean;
  onSelect: (id: string) => void;
}

/**
 * A row shows the last message rather than a status, so the presence dot is
 * the only sign of whether the person is around. It needs a word behind it for
 * anyone who cannot see the colour.
 */
function ConversationRow({ conversation, current, onSelect }: ConversationRowProps) {
  const { t } = useLanguage();
  const { peer, unread, time } = conversation;
  const presence = t(PRESENCE_LABEL[peer.presence]);

  return (
    <li>
      <button
        type="button"
        className={unread > 0 ? "dm-row unread" : "dm-row"}
        aria-current={current ? "page" : undefined}
        onClick={() => onSelect(conversation.id)}
      >
        <Avatar
          id={peer.id}
          displayName={peer.displayName}
          initials={peer.initials}
          avatarUrl={peer.avatarUrl}
          presence={peer.presence}
          title={presence}
        />
        <span className="dm-txt">
          {/* The name line also holds whatever marks the conversation itself
              before the name, so the name and the time stay on one row. */}
          <span className="dm-line">
            {conversation.encrypted ? (
              <span className="dm-row-lock" title={t("e2ee.lock")}>
                <Icon name="lock" size={13} />
                <span className="sr-only">{t("e2ee.lock")}</span>
              </span>
            ) : null}
            <b className="dm-name">{peer.displayName}</b>
            <span className="sr-only">{presence}</span>
            {time === undefined ? null : (
              <time className="dm-time" dateTime={time.iso}>
                {time.text}
              </time>
            )}
          </span>
          <span className="dm-line">
            <span className="dm-last">{conversation.preview}</span>
            {unread > 0 ? (
              <span className="ch-unread">
                {counted(unread)}
                <span className="sr-only">{t("shell.unreadConversation")}</span>
              </span>
            ) : conversation.callActive ? (
              <span className="dm-call-live" title={t("shell.activeCall")} aria-label={t("shell.activeCall")}>
                <Icon name="phone" size={14} />
              </span>
            ) : null}
          </span>
        </span>
      </button>
    </li>
  );
}

/**
 * The rail on the messages side of the app.
 *
 * The same column, with the same header and account row, as the server rail:
 * the two are one panel showing different contents. The server column beside
 * it is the way back, so the header only names the side and offers to start
 * a conversation.
 *
 * Friends is a row like a channel, because it is a place you go to, and the
 * conversations below it are people: a face, the last thing said, and when.
 */
export function DmRail({
  friendsActive,
  onOpenFriends,
  pending,
  onOpenActivities,
  personalActivity = null,
  onShowActivity,
  onCloseActivity,
  activityNotice = null,
  conversations,
  activeDmId,
  onSelect,
  onNewConversation,
  voiceDock,
  account,
  hidden = false,
}: DmRailProps) {
  const { t } = useLanguage();
  const searchId = useId();
  const [query, setQuery] = useState("");

  const typed = query.trim();
  const needle = typed.toLowerCase();
  const shown = conversations.filter((conversation) =>
    conversation.peer.displayName.toLowerCase().includes(needle),
  );

  return (
    <nav
      className="rail dm-rail"
      aria-label={t("shell.friendsAndMessages")}
      // `inert` alone keeps the drawer out of the tab order and the
      // accessibility tree. Toggling `aria-hidden` as well would warn about a
      // focused descendant when the click that closes the rail is still active.
      inert={hidden}
    >
      <div className="rail-header">
        <h1>{t("dm.messages")}</h1>
        <button
          type="button"
          className="rail-act"
          title={t("dm.newConversation")}
          aria-label={t("dm.newConversation")}
          onClick={onNewConversation}
        >
          <Icon name="edit" size={16} />
        </button>
      </div>

      <div className="rail-scroll">
        <div className="dm-find">
          <label className="sr-only" htmlFor={searchId}>
            {t("shell.findConversation")}
          </label>
          <span className="glyph" aria-hidden="true">
            <Icon name="search" size={15} />
          </span>
          <input
            id={searchId}
            type="search"
            value={query}
            placeholder={t("shell.findConversation")}
            autoComplete="off"
            onChange={(event) => setQuery(event.target.value)}
          />
        </div>

        <div className={friendsActive ? "channel active" : "channel"}>
          <button
            type="button"
            className="channel-main"
            aria-current={friendsActive ? "page" : undefined}
            onClick={onOpenFriends}
          >
            <span className="glyph" aria-hidden="true">
              <Icon name="users" size={16} />
            </span>
            <span className="name">{t("friends.title")}</span>
            {pending > 0 ? (
              <span className="ch-unread">
                {counted(pending)}
                <span className="sr-only">
                  {pending === 1
                    ? t("shell.pendingInvite.one")
                    : t("shell.pendingInvite.many")}
                </span>
              </span>
            ) : null}
          </button>
        </div>

        {onOpenActivities === undefined ? null : (
          <div className="channel">
            <button type="button" className="channel-main" aria-haspopup="dialog" onClick={onOpenActivities}>
              <span className="glyph" aria-hidden="true">
                <Icon name="game" size={16} />
              </span>
              <span className="name">{t("activities.title")}</span>
            </button>
          </div>
        )}

        {/* Your own activity is a place, like Friends: open, or in the
            background with a word saying so, and closed from its row. */}
        {personalActivity === null ? null : (
          <div className={personalActivity.shown ? "channel active dm-act" : "channel dm-act"}>
            <button
              type="button"
              className="channel-main"
              aria-current={personalActivity.shown ? "page" : undefined}
              onClick={onShowActivity}
            >
              <ActivityIcon
                install={personalActivity.install}
                name={personalActivity.name}
                icon={personalActivity.icon}
                size={16}
              />
              <span className="name">{personalActivity.name}</span>
              {personalActivity.shown ? null : <span className="dm-act-state">{t("activities.background")}</span>}
            </button>
            <button
              type="button"
              className="dm-act-close"
              aria-label={t("activities.closeNamed", { name: personalActivity.name })}
              title={t("activities.closeNamed", { name: personalActivity.name })}
              onClick={onCloseActivity}
            >
              <Icon name="x" size={14} />
            </button>
          </div>
        )}

        {activityNotice === null ? null : (
          <p className="dm-act-ended" role="status">
            {activityNotice}
          </p>
        )}

        <h2 className="group-caption">{t("shell.privateMessages")}</h2>

        {shown.length > 0 ? (
          <ul className="dm-list">
            {shown.map((conversation) => (
              <ConversationRow
                key={conversation.id}
                conversation={conversation}
                current={conversation.id === activeDmId}
                onSelect={onSelect}
              />
            ))}
          </ul>
        ) : (
          <p className="dm-empty">
            {conversations.length === 0
              ? t("shell.noConversations")
              : t("shell.noConversationMatch", { query: typed })}
          </p>
        )}
      </div>

      {voiceDock}
      {account}
    </nav>
  );
}
