import type { ApiClient } from "@/lib/api/client";
import type { RealtimeClient } from "@/lib/realtime/types";
import type { NotificationPort } from "@/ports";

export function createNotificationAdapter(
  api: ApiClient,
  realtime: RealtimeClient,
): NotificationPort {
  return {
    list: (options, signal) => api.listNotifications(options, signal),
    markRead: (notificationId, signal) => api.markNotificationRead(notificationId, signal),
    markAllRead: (signal) => api.markAllNotificationsRead(signal),
    delete: (notificationId, signal) => api.deleteNotification(notificationId, signal),
    onChanged(listener) {
      let previous = realtime.getStatus();
      const stopEvents = realtime.on("NotificationInboxChanged", listener);
      const stopStatus = realtime.subscribeStatus(() => {
        const next = realtime.getStatus();
        const recovered = next === "connected" && previous !== "connected";
        previous = next;
        if (recovered) listener({ global: true });
      });
      return () => {
        stopEvents();
        stopStatus();
      };
    },
  };
}
