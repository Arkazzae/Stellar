import { useCallback, useEffect, useState } from "react";

import type { EncryptionDeviceVM, EncryptionPaneProps } from "@/components/account/panes/EncryptionPane";
import { unsupportedReason } from "@/components/friends/encryptionCopy";
import { formatAgo, formatDate } from "@/lib/format/format";
import type { OwnDevice } from "@/ports";
import { useEncryption, useFriends, useLanguage, usePorts } from "@/state/hooks";

/** The encryption pane of the user settings, wired to the port and its dialogs. */
export function useEncryptionSettings(open: boolean): EncryptionPaneProps {
  const { encryption: port } = usePorts();
  const { status, openDialog } = useEncryption();
  const friends = useFriends();
  const { t, locale } = useLanguage();
  const [devices, setDevices] = useState<OwnDevice[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const hasKey = status.accountKey !== null;
  const { revision, deviceId } = status;

  const load = useCallback(() => {
    port.listDevices().then(
      (list) => {
        setDevices(list);
        setError(null);
      },
      () => setError(t("e2ee.settings.loadFailed")),
    );
  }, [port, t]);

  useEffect(() => {
    if (open && hasKey && status.support === "supported") load();
  }, [open, hasKey, status.support, revision, deviceId, load]);

  const now = new Date();
  const view: EncryptionDeviceVM[] | null =
    devices === null
      ? null
      : devices.map((device) => ({
          id: device.id,
          label: device.label,
          current: device.current,
          detail: device.current
            ? t("e2ee.settings.thisDevice", { date: formatDate(device.createdAt, locale) })
            : t("e2ee.settings.lastSeen", { ago: formatAgo(device.lastSeenAt, now, locale) }),
        }));

  return {
    status,
    unavailable: unsupportedReason(t, status.support),
    conversations: friends.conversations
      .filter((conversation) => conversation.encryption?.state === "on")
      .map((conversation) => conversation.peer.displayName),
    devices: view,
    error,
    busy,
    onSetUp: () => openDialog({ kind: "setup" }),
    onChangePin: () => openDialog({ kind: "newPin", mode: "change" }),
    onUnlock: () => openDialog({ kind: "unlock" }),
    onForgot: () => openDialog({ kind: "forgot" }),
    onRemoveDevice: (device) => {
      setBusy(true);
      setError(null);
      port
        .removeDevice(device.id)
        .then(load, () => setError(t("e2ee.settings.removeFailed")))
        .finally(() => setBusy(false));
    },
  };
}
