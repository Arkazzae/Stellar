import type { ApiClient } from "@/lib/api/client";
import { releaseDevicePush } from "@/lib/notifications/push";
import type { NotificationDeliveryPort } from "@/ports";

/** Account-global delivery settings over HTTP, plus cleanup of this device. */
export function createNotificationDeliveryAdapter(
  api: ApiClient,
): NotificationDeliveryPort {
  return {
    getCapabilities: (signal) => api.getNotificationDelivery(signal),
    savePushSubscription: (input, signal) => api.savePushSubscription(input, signal),
    deletePushSubscription: (endpoint, signal) =>
      api.deletePushSubscription(endpoint, signal),
    releaseDevice: () =>
      releaseDevicePush({
        deleteEndpoint: (endpoint) => api.deletePushSubscription(endpoint),
      }),
  };
}
