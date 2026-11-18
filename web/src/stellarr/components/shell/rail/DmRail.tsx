import { useId, useState, type ReactNode } from "react";

import { Icon } from "@stellarr/icons";
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
