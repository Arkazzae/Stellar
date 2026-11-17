import type { ApiClient } from "../client";
import { parseNotificationDelivery, parsePushSubscriptionSaved } from "../accountParsers";
import { getJson, sendJson, sendVoid } from "./http";

type NotificationDeliveryCalls = Pick<
  ApiClient,
  "getNotificationDelivery" | "savePushSubscription" | "deletePushSubscription"
>;

/**
 * Delivery settings belong to the account, not to the active server, so these
 * paths carry no server id of their own. The transport may still send the
 * selected server as a hint; the API ignores it here.
 */
export function notificationDeliveryCalls(): NotificationDeliveryCalls {
  return {
    getNotificationDelivery: (signal) =>
      getJson("/api/me/notification-delivery", parseNotificationDelivery, signal),

    savePushSubscription: (input, signal) =>
      sendJson("PUT", "/api/me/push-subscriptions", input, parsePushSubscriptionSaved, signal),

    deletePushSubscription: (endpoint, signal) =>
      sendVoid("DELETE", "/api/me/push-subscriptions", { endpoint }, signal),
  };
}
