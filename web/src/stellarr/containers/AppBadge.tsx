import { useEffect } from "react";
import { dismissActivityNotifications } from "@/lib/platform/badge";
import { useFriends, useNotifications, usePorts, useRuntime } from "@/state/hooks";

export function AppBadge() {
  const { unread } = useFriends();
  const { unreadCount } = useNotifications();
  const { realtime } = useRuntime();
  const { notifications } = usePorts();

  useEffect(() => {
    const numeric = typeof navigator.setAppBadge === "function";
    if (!numeric && (typeof ServiceWorkerRegistration === "undefined" ||
        !("getNotifications" in ServiceWorkerRegistration.prototype))) return;
    let request: AbortController | undefined;
    async function refresh() {
      request?.abort();
      const controller = new AbortController();
      request = controller;
      try {
        const response = await fetch("/api/me/app-badge", {
          credentials: "same-origin", cache: "no-store", redirect: "error", signal: controller.signal,
        });
        if (!response.ok) return;
        const { unread } = await response.json();
        if (controller.signal.aborted || !Number.isSafeInteger(unread) || unread < 0) return;
        if (unread > 0) {
          if (numeric) await navigator.setAppBadge(unread);
        } else {
          await navigator.clearAppBadge?.();
          if (!controller.signal.aborted) await dismissActivityNotifications();
        }
      } catch { /* Aborted, offline, or the OS declined the badge. */ }
    }
    void refresh();
    const unsubscribe = notifications.onChanged(refresh);
    const social = realtime.on("SocialChanged", refresh);
    const read = realtime.on("ChannelReadChanged", refresh);
    window.addEventListener("focus", refresh);
    window.addEventListener("online", refresh);
    return () => {
      request?.abort();
      unsubscribe();
      social();
      read();
      window.removeEventListener("focus", refresh);
      window.removeEventListener("online", refresh);
    };
  }, [notifications, realtime, unread, unreadCount]);
  return null;
}
