import { useState, type ReactNode } from "react";

import { Icon } from "@stellarr/icons";
import { AnchoredCard } from "@/components/primitives/AnchoredCard";
import type { MessageKey } from "@/i18n";
import { formatAgo, formatFull } from "@/lib/format/format";
import type { InboxNotification, ServerInviteNotificationTarget } from "@/ports";
import { useLanguage } from "@/state/hooks";

/** The source of a row written by the server when someone mentions the reader. */
const MENTION_PACKAGE_ID = "stellarr.mention";
/** An event's notices: the module says which, and the module's name is the event's. */
const EVENT_PACKAGE_ID = "stellarr.events";
const EVENT_NOTICES: Record<string, { title: MessageKey; body: MessageKey }> = {
  reminder: { title: "notifications.event.reminderTitle", body: "notifications.event.reminderBody" },
  seat: { title: "notifications.event.seatTitle", body: "notifications.event.seatBody" },
  cancelled: { title: "notifications.event.cancelledTitle", body: "notifications.event.cancelledBody" },
  changed: { title: "notifications.event.changedTitle", body: "notifications.event.changedBody" },
};

export interface NotificationInboxProps {
  items: readonly InboxNotification[];
  unreadCount: number;
  loading: boolean;
  loadingMore: boolean;
  hasOlder: boolean;
  error: string | null;
  anchor: HTMLElement;
  onClose: () => void;
  onReload: () => void;
  onLoadOlder: () => void;
  onMarkRead: (notificationId: string) => void;
  onMarkAllRead: () => void;
  onDelete: (notificationId: string) => void;
  /** The name of a channel the reader can see, or null when it is not known. */
  channelName: (channelId: string) => string | null;
  onOpenChannel: (channelId: string) => void;
  /** Accepts an invitation. Without it an invitation row has nothing to offer. */
  onJoinInvite?: (inviteCode: string) => Promise<void>;
  /**
   * The reader's scheduled messages from every place, as a second tab, with
   * how many there are. Without it the inbox has no tabs.
   */
  scheduled?: { count: number; list: ReactNode };
  /** Opens an event's card from a reminder or a seat given. */
  onOpenEvent?: (target: { serverId: string; eventId: string }) => void;
}

interface NotificationRowProps {
  notification: InboxNotification;
  channelName: (channelId: string) => string | null;
  /** The invitation being accepted right now, if any. */
  joiningInvite: string | null;
  onMarkRead: (notificationId: string) => void;
  onDelete: (notificationId: string) => void;
  onOpenChannel: (channelId: string) => void;
  onJoinInvite?: (inviteCode: string) => void;
  onOpenEvent?: (target: { serverId: string; eventId: string }) => void;
}

const MAX_UNREAD_SHOWN = 99;

function NotificationRow({
  notification,
  channelName,
  joiningInvite,
  onMarkRead,
  onDelete,
  onOpenChannel,
  onJoinInvite,
  onOpenEvent,
}: NotificationRowProps) {
  const { locale, t } = useLanguage();
  const { source, target } = notification;
  const unread = notification.readAt === undefined;
  const invite: ServerInviteNotificationTarget | null =
    target?.type === "server-invite" ? target : null;
  // A row opens a channel only when the reader can see it by name.
  const channelId = target?.type === "channel" ? target.channelId : null;
  const channel = channelId === null ? null : channelName(channelId);
  const canOpenChannel = channelId !== null && channel !== null;
  const canJoinInvite = invite !== null && onJoinInvite !== undefined;
  const isJoiningThis = invite !== null && joiningInvite === invite.inviteCode;
  const eventTarget = target?.type === "event" ? target : null;
  // An event's notices are worded here too, by which notice it is.
  const eventNotice = source.packageId === EVENT_PACKAGE_ID ? EVENT_NOTICES[source.moduleId] : undefined;

  const activateRow = canOpenChannel
    ? () => {
        if (unread) onMarkRead(notification.id);
        onOpenChannel(channelId);
      }
    : canJoinInvite
      ? () => {
          if (joiningInvite === null) onJoinInvite(invite.inviteCode);
        }
      : eventTarget !== null && onOpenEvent !== undefined
        ? () => {
            if (unread) onMarkRead(notification.id);
            onOpenEvent({ serverId: eventTarget.serverId, eventId: eventTarget.eventId });
          }
        : undefined;

  // Invitations and mentions are worded here, so that they follow the
  // reader's language; every other row arrives already written.
  let title = notification.title;
  let body = notification.body;
  if (invite !== null) {
    title = t("notifications.inviteTitle", { server: invite.serverName });
    body = t("notifications.inviteBody", {
      inviter: invite.invitedByName,
      server: invite.serverName,
    });
  } else if (source.packageId === MENTION_PACKAGE_ID) {
    title = t("notifications.mentionTitle", { author: source.moduleName });
  } else if (eventNotice !== undefined) {
    title = t(eventNotice.title, { event: source.moduleName });
    body = t(eventNotice.body);
  }

  const className = [
    "notification-item",
    unread && "unread",
    activateRow !== undefined && "actionable",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <li
      className={className}
      data-tone={notification.tone}
      onClick={(event) => {
        if (activateRow === undefined) return;
        const targetElement = event.target;
        if (
          targetElement instanceof Element &&
          targetElement.closest("button, a") !== null
        ) {
          return;
        }
        activateRow();
      }}
    >
      {/* No marker column. Unread is carried by the text and by the tick being
          there to press; a dot in a gutter on every row would be one more
          badge for a state the row already shows. */}
      <div className="notification-item-main">
        <p className="notification-title">
          <strong>{title}</strong>
          {unread ? <span className="sr-only">{t("notifications.new")}</span> : null}
        </p>
        <p className="notification-body">{body}</p>
        <div className="notification-foot">
          <span className="notification-meta">
            <span className="notification-source">
              {source.packageName}
              {source.moduleName === source.packageName ? null : ` · ${source.moduleName}`}
            </span>
            <span aria-hidden="true">·</span>
            <time
              dateTime={notification.createdAt}
              title={formatFull(notification.createdAt, locale)}
            >
              {formatAgo(notification.createdAt, new Date(), locale)}
            </time>
          </span>
          {!canOpenChannel ? null : (
            <button
              type="button"
              className="notification-open"
              // The chip shows the channel's name; the accessible name has to
              // say what pressing it does.
              aria-label={t("notifications.openChannel", { channel })}
              title={t("notifications.openChannel", { channel })}
              onClick={activateRow}
            >
              <Icon name="hash" size={12} />
              <span>{channel}</span>
            </button>
          )}
          {!canJoinInvite ? null : (
            <button
              type="button"
              className="notification-open primary"
              disabled={joiningInvite !== null}
              onClick={activateRow}
            >
              {isJoiningThis
                ? t("notifications.inviteJoining")
                : t("notifications.inviteJoin")}
            </button>
          )}
        </div>
      </div>
      {/* Housekeeping, not content: a tick and a bin beside the row rather
          than two more links competing with what it says. */}
      <div className="notification-tools">
        {unread ? (
          <button
            type="button"
            className="notification-tool"
            aria-label={t("notifications.markRead")}
            title={t("notifications.markRead")}
            onClick={() => onMarkRead(notification.id)}
          >
            <Icon name="tick" size={15} />
          </button>
        ) : null}
        <button
          type="button"
          className="notification-tool danger"
          aria-label={t("notifications.delete", { title })}
          title={t("notifications.deleteShort")}
          onClick={() => onDelete(notification.id)}
        >
          <Icon name="trash" size={14} />
        </button>
      </div>
    </li>
  );
}

/** The signed-in user's private inbox, as a card hanging off the bell. */
export function NotificationInbox({
  items,
  unreadCount,
  loading,
  loadingMore,
  hasOlder,
  error,
  anchor,
  onClose,
  onReload,
  onLoadOlder,
  onMarkRead,
  onMarkAllRead,
  onDelete,
  channelName,
  onOpenChannel,
  onJoinInvite,
  scheduled,
  onOpenEvent,
}: NotificationInboxProps) {
  const { t } = useLanguage();
  const [tab, setTab] = useState<"notifications" | "scheduled">("notifications");
  const onScheduled = scheduled !== undefined && tab === "scheduled";
  const [joiningInvite, setJoiningInvite] = useState<string | null>(null);
  const [inviteError, setInviteError] = useState<string | null>(null);

  const joinInvite =
    onJoinInvite === undefined
      ? undefined
      : (inviteCode: string) => {
          setInviteError(null);
          setJoiningInvite(inviteCode);
          onJoinInvite(inviteCode)
            .catch(() => setInviteError(t("notifications.inviteJoinFailed")))
            .finally(() => setJoiningInvite(null));
        };

  const message = error ?? inviteError;

  return (
    <AnchoredCard
      anchor={anchor}
      onClose={onClose}
      label={t("notifications.title")}
      className="notification-inbox"
    >
      <div className="notification-inbox-header">
        <div className="notification-inbox-heading">
          <h3>{t("notifications.title")}</h3>
          {unreadCount === 0 ? (
            <p aria-live="polite">{t("notifications.allRead")}</p>
          ) : (
            <span
              className="ch-unread notification-unread-count"
              aria-live="polite"
              aria-label={t("notifications.unreadCount", { count: unreadCount })}
            >
              {unreadCount > MAX_UNREAD_SHOWN ? `${MAX_UNREAD_SHOWN}+` : unreadCount}
            </span>
          )}
        </div>
        <div className="notification-inbox-head-actions">
          {unreadCount > 0 && !onScheduled ? (
            <button
              type="button"
              className="notification-mark-all"
              aria-label={t("notifications.markAllRead")}
              title={t("notifications.markAllRead")}
              onClick={onMarkAllRead}
            >
              <span className="notification-mark-all-icon" aria-hidden="true">
                <Icon name="tick" size={16} />
              </span>
              <span className="notification-mark-all-label">
                {t("notifications.markAllRead")}
              </span>
            </button>
          ) : null}
          <button
            type="button"
            className="notification-close"
            aria-label={t("common.close")}
            title={t("common.close")}
            onClick={onClose}
          >
            <Icon name="x" size={16} />
          </button>
        </div>
      </div>

      {scheduled === undefined ? null : (
        <div className="notification-tabs" role="tablist" aria-label={t("notifications.title")}>
          <button
            type="button"
            role="tab"
            className="notification-tab"
            aria-selected={!onScheduled}
            onClick={() => setTab("notifications")}
          >
            {t("notifications.title")}
          </button>
          <button
            type="button"
            role="tab"
            className="notification-tab"
            aria-selected={onScheduled}
            onClick={() => setTab("scheduled")}
          >
            {t("scheduled.title")}
            {scheduled.count === 0 ? null : <span>{scheduled.count}</span>}
          </button>
        </div>
      )}

      {onScheduled ? (
        <div role="tabpanel" className="notification-scheduled">
          {scheduled.list}
        </div>
      ) : null}

      {onScheduled || message === null ? null : (
        <div className="notification-error" role="alert">
          <span>{message}</span>
          {/* Only a failed load can be retried here; a refused invitation is
              retried by pressing its button again. */}
          {error === null ? null : (
            <button type="button" onClick={onReload}>
              {t("common.retry")}
            </button>
          )}
        </div>
      )}

      {onScheduled ? null : loading ? (
        <div className="notification-loading" aria-label={t("notifications.loading")}>
          {[0, 1, 2].map((placeholder) => (
            <div className="notification-skeleton" key={placeholder} aria-hidden="true">
              <span />
              <span />
              <span />
            </div>
          ))}
        </div>
      ) : items.length === 0 ? (
        <div className="notification-empty">
          <span className="notification-empty-icon" aria-hidden="true">
            <Icon name="bell" size={22} />
          </span>
          <strong>{t("notifications.emptyTitle")}</strong>
          <p>{t("notifications.emptyBody")}</p>
        </div>
      ) : (
        <ol className="notification-list">
          {items.map((notification) => (
            <NotificationRow
              key={notification.id}
              notification={notification}
              channelName={channelName}
              joiningInvite={joiningInvite}
              onMarkRead={onMarkRead}
              onDelete={onDelete}
              onOpenChannel={onOpenChannel}
              onJoinInvite={joinInvite}
              {...(onOpenEvent === undefined ? {} : { onOpenEvent })}
            />
          ))}
        </ol>
      )}

      {!onScheduled && !loading && hasOlder ? (
        <button
          type="button"
          className="notification-load-older"
          disabled={loadingMore}
          onClick={onLoadOlder}
        >
          {loadingMore ? t("notifications.loadingOlder") : t("notifications.loadOlder")}
        </button>
      ) : null}
    </AnchoredCard>
  );
}
