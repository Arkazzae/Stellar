import { useCallback, useEffect, useRef, useState } from "react";

import type { NotificationDeliveryProps } from "@/components/account/panes/NotificationsPane";
import type { MessageKey } from "@/i18n";
import {
  PushEnrollmentError,
  browserIsBrave,
  decodeApplicationServerKey,
  deviceNotificationPermission,
  disableDevicePush,
  enrollDevicePush,
  inspectDevicePush,
  pushFailureDetail,
  pushManagerSupported,
  requestDeviceNotificationPermission,
} from "@/lib/notifications/push";
import type { NotificationDeliveryCapabilities } from "@/ports";
import { useAccount, usePorts, useSession } from "@/state/hooks";

const IDLE_DEVICE: NotificationDeliveryProps["device"] = {
  supported: false,
  permission: "unsupported",
  status: "idle",
  pending: false,
  error: null,
  errorDetail: null,
};

/**
 * What went wrong, in words that say what to do about it. Null for an
 * enrollment nobody waits for any more (a second click, a sign-out, another
 * account): it was let go, nothing failed.
 */
function failureMessage(cause: unknown): MessageKey | null {
  const code = cause instanceof PushEnrollmentError ? cause.code : "subscribe";
  switch (code) {
    case "unsupported":
      return "preference.pushDeviceUnsupported";
    case "invalid-key":
      return "preference.pushDeviceKeyFailed";
    case "worker":
      return "preference.pushDeviceWorkerFailed";
    case "push-service":
      return browserIsBrave()
        ? "preference.pushDeviceServiceBrave"
        : "preference.pushDeviceServiceFailed";
    case "denied":
      return "preference.pushDeviceDenied";
    case "key-mismatch":
      return "preference.pushDeviceKeyMismatch";
    case "timeout":
      return "preference.pushDeviceTimeout";
    case "malformed":
      return "preference.pushDeviceMalformed";
    case "server":
      return "preference.pushDeviceServerFailed";
    case "cancelled":
      return null;
    case "subscribe":
      return "preference.pushDeviceSubscribeFailed";
  }
}

/**
 * Owns what the notifications pane may show and do about delivery.
 *
 * Every asynchronous answer is checked against the account and the operation
 * that started it: a subscription that finishes after a sign-out, a new user,
 * or a second click is either discarded or rolled back, never applied to the
 * wrong person. The account's global switches and the device's enrollment are
 * separate actions, and a failed enrollment changes no stored preference.
 */
export function useNotificationDelivery(open: boolean): NotificationDeliveryProps | null {
  const ports = usePorts();
  const account = useAccount();
  const session = useSession();
  const port = ports.notificationDelivery;
  const userId = session.user?.id ?? null;

  const [capabilities, setCapabilities] = useState<NotificationDeliveryCapabilities | null>(null);
  const [loading, setLoading] = useState(false);
  const [loadFailed, setLoadFailed] = useState(false);
  const [reloadKey, setReloadKey] = useState(0);
  const [savingEmail, setSavingEmail] = useState(false);
  const [savingPush, setSavingPush] = useState(false);
  const [emailError, setEmailError] = useState<MessageKey | null>(null);
  const [pushError, setPushError] = useState<MessageKey | null>(null);
  const [device, setDevice] = useState<NotificationDeliveryProps["device"]>(() => ({
    ...IDLE_DEVICE,
    supported: pushManagerSupported(),
    permission: deviceNotificationPermission(),
  }));

  const mounted = useRef(true);
  const currentUser = useRef(userId);
  currentUser.current = userId;
  const emailRequest = useRef(0);
  const pushRequest = useRef(0);
  const deviceRequest = useRef(0);

  const isCurrent = useCallback(
    (target: string) => mounted.current && currentUser.current === target,
    [],
  );

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  // A different account invalidates every answer that belonged to the old one.
  useEffect(() => {
    emailRequest.current += 1;
    pushRequest.current += 1;
    deviceRequest.current += 1;
    setEmailError(null);
    setPushError(null);
    setDevice((current) => ({
      ...current,
      pending: false,
      error: null,
      errorDetail: null,
      status: "idle",
    }));
  }, [userId]);

  useEffect(() => {
    if (!open || port === undefined || userId === null) return;
    const controller = new AbortController();
    let active = true;
    setLoading(true);
    setLoadFailed(false);
    port.getCapabilities(controller.signal).then(
      (answer) => {
        if (!active || !isCurrent(userId)) return;
        setCapabilities(answer);
        setLoading(false);
      },
      () => {
        if (!active || !isCurrent(userId)) return;
        setCapabilities(null);
        setLoading(false);
        setLoadFailed(true);
      },
    );
    return () => {
      active = false;
      controller.abort();
    };
  }, [isCurrent, open, port, reloadKey, userId]);

  const refreshDevice = useCallback(async () => {
    const target = currentUser.current;
    if (target === null) return;
    if (!pushManagerSupported()) {
      if (!isCurrent(target)) return;
      setDevice({ ...IDLE_DEVICE });
      return;
    }
    const inspection = await inspectDevicePush(target);
    if (!isCurrent(target)) return;
    setDevice((current) => ({
      ...current,
      supported: inspection.state !== "unsupported",
      permission: deviceNotificationPermission(),
      status:
        inspection.state === "enrolled"
          ? "enrolled"
          : inspection.state === "stale"
            ? "stale"
            : "idle",
    }));
  }, [isCurrent]);

  useEffect(() => {
    if (!open || port === undefined || userId === null) return;
    void refreshDevice();
  }, [open, port, refreshDevice, reloadKey, userId]);

  // Permission or registration can change outside the app; look again when the
  // person comes back, without ever registering or prompting on their behalf.
  useEffect(() => {
    if (!open) return;
    const refresh = () => void refreshDevice();
    window.addEventListener("focus", refresh);
    document.addEventListener("visibilitychange", refresh);
    return () => {
      window.removeEventListener("focus", refresh);
      document.removeEventListener("visibilitychange", refresh);
    };
  }, [open, refreshDevice]);

  const changeEmail = useCallback(
    (enabled: boolean) => {
      const target = currentUser.current;
      if (port === undefined || target === null || capabilities === null) return;
      if (!capabilities.emailAvailable || !capabilities.emailVerified) return;
      const request = ++emailRequest.current;
      setSavingEmail(true);
      setEmailError(null);
      void account.setPreferences({ emailNotifications: enabled }).then(
        () => {
          if (!isCurrent(target) || request !== emailRequest.current) return;
          setSavingEmail(false);
        },
        () => {
          if (!isCurrent(target) || request !== emailRequest.current) return;
          setSavingEmail(false);
          setEmailError("preference.emailFailed");
        },
      );
    },
    [account, capabilities, isCurrent, port],
  );

  const changePush = useCallback(
    (enabled: boolean) => {
      const target = currentUser.current;
      if (port === undefined || target === null || capabilities === null) return;
      if (!capabilities.pushAvailable || capabilities.applicationServerKey === null) return;
      const request = ++pushRequest.current;
      setSavingPush(true);
      setPushError(null);
      void account.setPreferences({ pushNotifications: enabled }).then(
        () => {
          if (!isCurrent(target) || request !== pushRequest.current) return;
          setSavingPush(false);
        },
        () => {
          if (!isCurrent(target) || request !== pushRequest.current) return;
          setSavingPush(false);
          setPushError("preference.pushFailed");
        },
      );
    },
    [account, capabilities, isCurrent, port],
  );

  const enableDevice = useCallback(() => {
    const target = currentUser.current;
    if (port === undefined || target === null) return;

    if (!pushManagerSupported()) {
      setDevice((current) => ({
        ...current,
        supported: false,
        permission: "unsupported",
        pending: false,
        error: "preference.pushDeviceUnsupported",
        errorDetail: null,
      }));
      return;
    }
    if (
      capabilities === null ||
      !capabilities.pushAvailable ||
      capabilities.applicationServerKey === null
    ) {
      setDevice((current) => ({
        ...current,
        pending: false,
        error: "preference.pushDeviceUnavailable",
        errorDetail: null,
      }));
      return;
    }

    // Both checks above and the key decoding are synchronous: a capability or
    // key problem must be found before the browser is asked for permission.
    let key: Uint8Array<ArrayBuffer>;
    try {
      key = decodeApplicationServerKey(capabilities.applicationServerKey);
    } catch (cause) {
      setDevice((current) => ({
        ...current,
        pending: false,
        error: "preference.pushDeviceKeyFailed",
        errorDetail: pushFailureDetail(cause),
      }));
      return;
    }

    const request = ++deviceRequest.current;
    const stillWanted = () => isCurrent(target) && request === deviceRequest.current;
    setDevice((current) => ({
      ...current,
      supported: true,
      permission: deviceNotificationPermission(),
      pending: true,
      error: null,
      errorDetail: null,
    }));

    const permissionFailed = (cause: unknown) =>
      setDevice((current) => ({
        ...current,
        pending: false,
        permission: deviceNotificationPermission(),
        error: "preference.pushDevicePermissionFailed",
        errorDetail: pushFailureDetail(cause),
      }));

    // Started directly in the click, before the first `await`: browsers only
    // show the permission prompt for a request made inside the gesture.
    let permissionAnswer: Promise<NotificationPermission | "unsupported">;
    try {
      permissionAnswer = requestDeviceNotificationPermission();
    } catch (cause) {
      permissionFailed(cause);
      return;
    }

    void (async () => {
      let permission: NotificationPermission | "unsupported";
      try {
        permission = await permissionAnswer;
      } catch (cause) {
        // A refused request (an insecure page, a policy) is about permission,
        // not about the push service that was never reached.
        if (stillWanted()) permissionFailed(cause);
        return;
      }
      if (!stillWanted()) return;
      if (permission !== "granted") {
        setDevice((current) => ({
          ...current,
          pending: false,
          permission: permission === "unsupported" ? "unsupported" : permission,
          error:
            permission === "unsupported"
              ? "preference.pushDeviceUnsupported"
              : "preference.pushDevicePermissionFailed",
          errorDetail: null,
        }));
        return;
      }

      try {
        await enrollDevicePush({
          userId: target,
          applicationServerKey: key,
          save: (input) => port.savePushSubscription(input),
          deleteEndpoint: (endpoint) => port.deletePushSubscription(endpoint),
          isStillWanted: stillWanted,
        });
        if (!stillWanted()) return;
        setDevice((current) => ({
          ...current,
          pending: false,
          permission: deviceNotificationPermission(),
          status: "enrolled",
          error: null,
          errorDetail: null,
        }));
      } catch (cause) {
        if (!stillWanted()) return;
        const error = failureMessage(cause);
        setDevice((current) => ({
          ...current,
          pending: false,
          permission: deviceNotificationPermission(),
          error,
          errorDetail: error === null ? null : pushFailureDetail(cause),
        }));
      }
    })();
  }, [capabilities, isCurrent, port]);

  const disableDevice = useCallback(() => {
    const target = currentUser.current;
    if (port === undefined || target === null) return;
    const request = ++deviceRequest.current;
    const stillWanted = () => isCurrent(target) && request === deviceRequest.current;
    setDevice((current) => ({ ...current, pending: true, error: null, errorDetail: null }));

    void disableDevicePush({
      userId: target,
      deleteEndpoint: (endpoint) => port.deletePushSubscription(endpoint),
    }).then(
      (result) => {
        if (!stillWanted()) return;
        setDevice((current) => ({
          ...current,
          pending: false,
          permission: deviceNotificationPermission(),
          status: "idle",
          error: result.serverRemoved ? null : "preference.pushDeviceDeleteFailed",
          errorDetail: null,
        }));
      },
      (cause: unknown) => {
        if (!stillWanted()) return;
        setDevice((current) => ({
          ...current,
          pending: false,
          permission: deviceNotificationPermission(),
          status: "idle",
          error: "preference.pushDeviceDeleteFailed",
          errorDetail: pushFailureDetail(cause),
        }));
      },
    );
  }, [isCurrent, port]);

  const reload = useCallback(() => setReloadKey((key) => key + 1), []);

  if (port === undefined || userId === null) return null;

  return {
    // The first render after opening happens before the effect starts the
    // request; it must say "checking", not "unavailable", about a server that
    // has not answered yet.
    loading: loading || (open && capabilities === null && !loadFailed),
    loadFailed,
    emailAvailable: capabilities?.emailAvailable ?? false,
    emailVerified: capabilities?.emailVerified ?? false,
    pushAvailable: capabilities?.pushAvailable ?? false,
    pushKeyAvailable: capabilities?.applicationServerKey !== null && capabilities?.applicationServerKey !== undefined,
    emailEnabled: account.preferences.emailNotifications === true,
    pushEnabled: account.preferences.pushNotifications === true,
    savingEmail,
    savingPush,
    emailError,
    pushError,
    device,
    onEmailChange: changeEmail,
    onPushChange: changePush,
    onEnableDevice: enableDevice,
    onDisableDevice: disableDevice,
    onReload: reload,
  };
}
