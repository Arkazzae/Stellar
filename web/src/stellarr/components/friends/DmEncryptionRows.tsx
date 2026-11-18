import { Switch } from "@/components/primitives/Switch";
import { useLanguage } from "@/state/LanguageProvider";

import { ProfileAction } from "./DmProfilePanel";

/** The switch as the reader sees it: theirs to answer is "asked", theirs to wait on is "waiting". */
export type EncryptionSwitchView = "off" | "waiting" | "asked" | "on";

export interface DmEncryptionRowsProps {
  peerName: string;
  state: EncryptionSwitchView;
  /** Formatted date it was switched on, while it is on. */
  enabledOn?: string;
  /** Why this browser cannot take part; the switch is disabled and says so. */
  unavailable: string | null;
  busy: boolean;
  error: string | null;
  /** The safety code's standing, while encryption is on. */
  code?: { status: "unverified" | "changed" } | { status: "verified"; on: string };
  onSwitch(on: boolean): void;
  onOpenCode(): void;
}

/**
 * The top of the profile panel's actions in a conversation: the end-to-end
 * switch, which says where it stands and what it is waiting for, and, while
 * it is on, the safety code.
 */
export function DmEncryptionRows({
  peerName,
  state,
  enabledOn,
  unavailable,
  busy,
  error,
  code,
  onSwitch,
  onOpenCode,
}: DmEncryptionRowsProps) {
  const { t } = useLanguage();
  const checked = state === "on" || state === "waiting";

  const detail =
    error !== null ? (
      <span className="pp-tone bad">{error}</span>
    ) : unavailable !== null ? (
      unavailable
    ) : state === "waiting" ? (
      <span className="pp-tone warn">{t("e2ee.switch.waiting", { name: peerName })}</span>
    ) : state === "asked" ? (
      <span className="pp-tone warn">{t("e2ee.switch.asked", { name: peerName })}</span>
    ) : state === "on" ? (
      <span className="pp-tone ok">{t("e2ee.switch.on", { date: enabledOn ?? "" })}</span>
    ) : (
      t("e2ee.switch.off")
    );

  return (
    <>
      <ProfileAction
        icon="lock"
        label={t("e2ee.switch.title")}
        detail={detail}
        trailing={
          <Switch
            checked={checked}
            onChange={onSwitch}
            label={t("e2ee.switch.title")}
            // A browser that cannot encrypt takes no part in the switch at all.
            disabled={busy || unavailable !== null}
          />
        }
      />
      {state === "on" && code !== undefined ? (
        <ProfileAction
          icon="key"
          label={t("e2ee.code.title")}
          detail={
            code.status === "verified" ? (
              <span className="pp-tone ok">{t("e2ee.code.verified", { date: code.on })}</span>
            ) : code.status === "changed" ? (
              <span className="pp-tone warn">{t("e2ee.code.changed")}</span>
            ) : (
              t("e2ee.code.unverified")
            )
          }
          onClick={() => onOpenCode()}
        />
      ) : null}
    </>
  );
}
