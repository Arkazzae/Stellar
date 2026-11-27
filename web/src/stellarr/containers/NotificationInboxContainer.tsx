import { useCallback, useState } from "react";

import { eventWindow } from "@/lib/events/eventSheetRequest";
import { offerInstallAfterInvite } from "@/lib/platform/install";
import { Icon } from "@stellarr/icons";
import { ScheduledList } from "@/components/scheduled/ScheduledList";
import { NotificationInbox } from "@/components/shell/NotificationInbox";
import {
  useDirectory,
  useLanguage,
  useNotifications,
  usePorts,
  useSession,
  useUi,
} from "@/state/hooks";

import { useReaderZone, useScheduledItems, useScheduledListActions } from "./useScheduled";

const BADGE_LIMIT = 99;

/** The inbox's second tab: every scheduled message, from every place. */
function ScheduledInboxList({ now }: { now: Date }) {
  const items = useScheduledItems();
  const actions = useScheduledListActions();
  const zone = useReaderZone();
  return <ScheduledList compact items={items} now={now} zone={zone} {...actions} />;
}

/**
 * The bell in a header and the inbox it opens, anchored to the bell itself.
 * The inbox is mounted only while open, so a closed bell costs nothing.
 */
export function NotificationInboxButton() {
  const inbox = useNotifications();
  const directory = useDirectory();
  const ui = useUi();
  const session = useSession();
  const { t } = useLanguage();
  const [button, setButton] = useState<HTMLButtonElement | null>(null);
  const [clicked, setAnchor] = useState<HTMLButtonElement | null>(null);
  const scheduledCount = useScheduledItems().length;
  const ports = usePorts();
  /** When the inbox opened, which the scheduled list's "today" is read against. */
  const [openedAt, setOpenedAt] = useState(() => new Date());
  const anchor = ui.modal === "inbox" ? button : clicked;

  // Stable, because the inbox repositions and re-listens whenever it changes.
  const { modal, closeModal } = ui;
  const close = useCallback(() => {
    setAnchor(null);
    if (modal === "inbox") closeModal();
  }, [modal, closeModal]);
  const label =
    inbox.unreadCount > 0
      ? t("notifications.openUnread", { count: inbox.unreadCount })
      : t("notifications.open");

  const joinInvite = async (inviteCode: string) => {
    const server = await directory.joinInvite(inviteCode);
    offerInstallAfterInvite();
    directory.selectServer(server.id);
    ui.openServer();
    session.reload();
    close();
  };

  return (
    <>
      <button
        ref={setButton}
        type="button"
        className="pill-btn notification-trigger"
        title={label}
        aria-label={label}
        aria-haspopup="dialog"
        aria-expanded={anchor !== null}
        onClick={(event) => {
          const bell = event.currentTarget;
          if (anchor) close();
          else {
            setOpenedAt(new Date());
            setAnchor(bell);
          }
        }}
      >
        <Icon name="bell" size={17} />
        {inbox.unreadCount > 0 ? (
          <span className="notification-badge" aria-hidden="true">
            {inbox.unreadCount > BADGE_LIMIT ? `${BADGE_LIMIT}+` : inbox.unreadCount}
          </span>
        ) : null}
      </button>
      {anchor === null ? null : (
        <NotificationInbox
          items={inbox.items}
          unreadCount={inbox.unreadCount}
          loading={inbox.loading}
          loadingMore={inbox.loadingMore}
          hasOlder={inbox.hasOlder}
          error={inbox.error}
          anchor={anchor}
          onClose={close}
          onReload={inbox.reload}
          onLoadOlder={() => void inbox.loadOlder()}
          onMarkRead={(id) => void inbox.markRead(id)}
          onMarkAllRead={() => void inbox.markAllRead()}
          onDelete={(id) => void inbox.remove(id)}
          channelName={(channelId) =>
            directory.textChannels.find((channel) => channel.id === channelId)?.name ?? null
          }
          onOpenChannel={(channelId) => {
            ui.selectTextChannel(channelId);
            close();
          }}
          onJoinInvite={joinInvite}
          scheduled={{ count: scheduledCount, list: <ScheduledInboxList now={openedAt} /> }}
          onOpenEvent={(eventId) => {
            // The card opens over the server's room, where its events are known.
            if (ports.events.get(eventId) === undefined) return;
            eventWindow.open({ kind: "view", eventId });
            close();
          }}
        />
      )}
    </>
  );
}
