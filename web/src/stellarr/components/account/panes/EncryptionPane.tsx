import { Button } from "@stellarr/ui";
import { useConfirm } from "@/components/primitives/ConfirmDialog";
import type { EncryptionStatus } from "@/ports";
import { useLanguage } from "@/state/LanguageProvider";

/** One browser that holds keys, already worded. */
export interface EncryptionDeviceVM {
  id: string;
  label: string;
  /** "This browser. Keys since…" or "Last seen…". */
  detail: string;
  current: boolean;
}

export interface EncryptionPaneProps {
  status: EncryptionStatus;
  /** Why this browser cannot encrypt, when it cannot. */
  unavailable: string | null;
  /** Names of the people whose conversations with you are encrypted. */
  conversations: string[];
  /** Null while loading. */
  devices: EncryptionDeviceVM[] | null;
  error: string | null;
  busy: boolean;
  onSetUp(): void;
  onChangeRecoveryCode(): void;
  onUnlock(): void;
  onForgot(): void;
  onRemoveDevice(device: EncryptionDeviceVM): void;
}

/**
 * The account's encryption: the recovery code that protects the key backup, which
 * conversations are encrypted, and every browser holding keys, each of which
 * can lose them here.
 */
export function EncryptionPane({
  status,
  unavailable,
  conversations,
  devices,
  error,
  busy,
  onSetUp,
  onChangeRecoveryCode,
  onUnlock,
  onForgot,
  onRemoveDevice,
}: EncryptionPaneProps) {
  const { t } = useLanguage();
  const confirm = useConfirm();
  const hasKey = status.accountKey !== null;

  const recoveryDetail = !hasKey
    ? t("e2ee.settings.pinNone")
    : status.migrationRequired && (status.unlocked || status.backup !== null) ? t(status.unlocked ? "e2ee.migrate.detail" : "e2ee.migrate.unlock")
    : !status.unlocked
      ? t(status.backup === null ? "e2ee.unlock.missing" : "e2ee.settings.pinLocked")
      : status.backup === null
        ? t("e2ee.settings.pinNoBackup")
        : t("e2ee.settings.pinDetail");

  const recoveryAction =
    unavailable !== null ? null : !hasKey ? (
      <Button type="button" variant="secondary" size="sm" onClick={onSetUp}>
        {t("e2ee.settings.set")}
      </Button>
    ) : status.unlocked ? (
      <Button type="button" variant="secondary" size="sm" onClick={onChangeRecoveryCode}>
        {t("e2ee.settings.change")}
      </Button>
    ) : (
      <span className="enc-row-acts">
        {status.backup === null ? null : (
          <Button type="button" variant="secondary" size="sm" onClick={onUnlock}>
            {t("e2ee.settings.unlock")}
          </Button>
        )}
        <Button type="button" variant="quiet" size="sm" onClick={onForgot}>
          {t("e2ee.settings.forgot")}
        </Button>
      </span>
    );

  const remove = async (device: EncryptionDeviceVM) => {
    const confirmed = await confirm({
      title: t("e2ee.settings.removeTitle"),
      description: t("e2ee.settings.removeDetail"),
      subject: { kind: "item", name: device.label, detail: device.detail, mark: { icon: "key" } },
      confirmLabel: t("e2ee.settings.remove"),
      danger: true,
    });
    if (confirmed) onRemoveDevice(device);
  };

  return (
    <>
      {unavailable === null ? null : (
        <p className="set-note" role="note">
          {unavailable}
        </p>
      )}
      {error === null ? null : (
        <p className="acc-answer bad" role="alert">
          {error}
        </p>
      )}
      <section className="sc acc-security">
        <header className="sc-h">{t("e2ee.settings.section")}</header>
        <div className="sc-b acc-security-list">
          <div className="acc-security-item">
            <div className="acc-security-summary">
              <span className="acc-security-copy">
                <b>{t("e2ee.settings.pin")}</b>
                <small>{recoveryDetail}</small>
              </span>
              {recoveryAction}
            </div>
          </div>
          <div className="acc-security-item">
            <div className="acc-security-summary">
              <span className="acc-security-copy">
                <b>{t("e2ee.settings.conversations")}</b>
                <small>
                  {conversations.length === 0
                    ? t("e2ee.settings.conversationsNone")
                    : t("e2ee.settings.conversationsDetail", { names: conversations.join(", ") })}
                </small>
              </span>
            </div>
          </div>
        </div>
      </section>

      {hasKey ? (
        <section className="sc acc-security">
          <header className="sc-h">{t("e2ee.settings.devices")}</header>
          <div className="sc-b acc-security-list">
            {devices === null ? null : devices.length === 0 ? (
              <p className="acc-security-item acc-security-empty">{t("e2ee.settings.devicesEmpty")}</p>
            ) : (
              devices.map((device) => (
                <div className="acc-security-item" key={device.id}>
                  <div className="acc-security-summary">
                    <span className="acc-security-copy">
                      <b>{device.label}</b>
                      <small>{device.detail}</small>
                    </span>
                    {device.current ? null : (
                      <Button
                        type="button"
                        variant="danger"
                        size="sm"
                        disabled={busy}
                        onClick={() => void remove(device)}
                      >
                        {t("e2ee.settings.remove")}
                      </Button>
                    )}
                  </div>
                </div>
              ))
            )}
          </div>
        </section>
      ) : null}
    </>
  );
}
