/**
 * Account-global notification delivery: what the server can send (e-mail and
 * Web Push) and the browser subscriptions that make push possible.
 *
 * These routes are deliberately not tied to the active server: a direct
 * message and a mention are the person's, wherever they happen.
 */

export interface NotificationDeliveryCapabilities {
  /** Whether this instance can send notification e-mail at all. */
  emailAvailable: boolean;
  /** Whether the account's address was confirmed as an e-mail second step. */
  emailVerified: boolean;
  /** Whether this instance has Web Push configured. */
  pushAvailable: boolean;
  /** The VAPID public key; null when Web Push is not configured. */
  applicationServerKey: string | null;
}

export interface PushSubscriptionKeys {
  p256dh: string;
  auth: string;
}

export interface PushSubscriptionInput {
  endpoint: string;
  keys: PushSubscriptionKeys;
}

export interface NotificationDeliveryPort {
  getCapabilities(signal?: AbortSignal): Promise<NotificationDeliveryCapabilities>;
  /** Binds this browser's subscription to the current user and session. */
  savePushSubscription(input: PushSubscriptionInput, signal?: AbortSignal): Promise<string>;
  /** Forgets the server's row for an endpoint; the browser subscription stays. */
  deletePushSubscription(endpoint: string, signal?: AbortSignal): Promise<void>;
  /**
   * Releases this browser's subscription: asks the server to forget the
   * endpoint while the session is still valid, then unsubscribes locally.
   * Local removal happens even when the server request fails.
   */
  releaseDevice(): Promise<void>;
}
