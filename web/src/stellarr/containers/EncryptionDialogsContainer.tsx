import { useEffect, useRef, useState } from "react";

import { SecurityStepDialog } from "@/components/account/panes/SecurityStepDialog";
import {
  ForgotPinDialog,
  NewPinDialog,
  PinSetupDialog,
  SafetyCodeDialog,
  UnlockDialog,
} from "@/components/friends/EncryptionDialogs";
import { ApiError } from "@/lib/api/errors";
import { E2eeError } from "@/lib/e2ee/client";
import { pluralForm } from "@/lib/format/format";
import type { MessageKey } from "@/i18n";
import type { PreparedPin, SafetyCodeView, UnlockResult } from "@/ports";
import { useEncryption, useFriends, useLanguage, usePorts, useSession } from "@/state/hooks";

import { FRESH_SECOND_STEP_REQUIRED } from "./freshSecondStep";
import { toPerson } from "./toPerson";
import { useAccountSecurity } from "./useAccountSecurity";

const WRONG_PIN: Record<"one" | "few" | "many", MessageKey> = {
  one: "e2ee.unlock.wrong.one",
  few: "e2ee.unlock.wrong.few",
  many: "e2ee.unlock.wrong.many",
};

/**
 * Every encryption dialog, raised from wherever the reader is: the first PIN
 * (then the switch it was for), unlocking history with the PIN, what
 * forgetting it costs, a new PIN or new keys behind the same re-authentication
 * as a password change, and the safety code.
 */
export function EncryptionDialogsContainer() {
  const { encryption: port } = usePorts();
  const { dialog, closeDialog, openDialog, runSwitch } = useEncryption();
  const friends = useFriends();
  const session = useSession();
  const { t, locale } = useLanguage();
  // Its own instance: a new PIN raised from a conversation needs the same
  // second step the settings ask for, with the settings shut.
  const security = useAccountSecurity(false);

  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [passwordError, setPasswordError] = useState<string | null>(null);
  const [dead, setDead] = useState(false);
  const [code, setCode] = useState<SafetyCodeView | null>(null);
  const [mismatch, setMismatch] = useState(false);
  /** The slow PIN derivation is done once per PIN, however many times re-authentication retries. */
  const prepared = useRef<{ key: string; value: PreparedPin } | null>(null);

  const kind = dialog?.kind ?? null;
  const [lastKind, setLastKind] = useState(kind);
  if (lastKind !== kind) {
    setLastKind(kind);
    setPending(false);
    setError(null);
    setPasswordError(null);
    setDead(false);
    setMismatch(false);
    setCode(null);
  }

  const safetyId = dialog?.kind === "safety" ? dialog.conversationId : null;
  useEffect(() => {
    if (safetyId === null) return;
    let current = true;
    port.safetyCode(safetyId).then(
      (view) => {
        if (!current) return;
        if (view === null) setError(t("e2ee.safety.failed"));
        else setCode(view);
      },
      () => current && setError(t("e2ee.safety.failed")),
    );
    return () => {
      current = false;
    };
  }, [port, safetyId, t]);

  const close = () => {
    prepared.current = null;
    closeDialog();
  };

  const setUp = (pin: string) => {
    if (dialog?.kind !== "setup") return;
    const then = dialog.then;
    setPending(true);
    setError(null);
    port.setUp(pin).then(
      () => {
        close();
        if (then !== undefined) runSwitch(then.conversationId, then.action);
      },
      () => {
        setPending(false);
        setError(t("e2ee.setup.failed"));
      },
    );
  };

  const describeUnlock = (result: Exclude<UnlockResult, { kind: "unlocked" }>): string => {
    switch (result.kind) {
      case "wrong-pin":
        return t(WRONG_PIN[pluralForm(locale, result.remainingAttempts)], { count: result.remainingAttempts });
      case "backup-deleted":
        return t("e2ee.unlock.deleted");
      case "backup-missing":
        return t("e2ee.unlock.missing");
      case "rate-limited":
        return t("e2ee.unlock.rateLimited");
      case "backup-unusable":
        return t("e2ee.unlock.unusable");
    }
  };

  const unlock = (pin: string) => {
    setPending(true);
    setError(null);
    port.unlock(pin).then(
      (result) => {
        setPending(false);
        if (result.kind === "unlocked") {
          close();
          return;
        }
        setError(describeUnlock(result));
        setDead(result.kind === "backup-deleted" || result.kind === "backup-missing" || result.kind === "backup-unusable");
      },
      () => {
        setPending(false);
        setError(t("e2ee.unlock.failed"));
      },
    );
  };

  const newPin = async ({ password, pin }: { password: string; pin: string }) => {
    if (dialog?.kind !== "newPin") return;
    const { mode } = dialog;
    setPending(true);
    setError(null);
    setPasswordError(null);
    try {
      const key = `${mode}:${pin}`;
      if (prepared.current?.key !== key) {
        prepared.current = {
          key,
          value: mode === "change" ? await port.prepareNewPin(pin) : await port.prepareReset(pin),
        };
      }
      const value = prepared.current.value;
      const attempt = () =>
        mode === "change"
          ? port.changePin(value, password)
          : port.reset(value as Parameters<typeof port.reset>[0], password);
      try {
        await attempt();
      } catch (cause) {
        if (!(cause instanceof ApiError) || cause.code !== FRESH_SECOND_STEP_REQUIRED) throw cause;
        // The same proof a password change asks for, then the same request again.
        if (!(await security.confirmSecondStep())) {
          setError(t("e2ee.newPin.secondStep"));
          setPending(false);
          return;
        }
        await attempt();
      }
      close();
    } catch (cause) {
      setPending(false);
      if (cause instanceof ApiError && cause.code === "password-incorrect") {
        setPasswordError(t("e2ee.password.wrong"));
      } else if (cause instanceof E2eeError && cause.reason === "key-changed") {
        setError(t("e2ee.newPin.keyChanged"));
      } else if (cause instanceof ApiError && cause.kind === "rate-limited") {
        setError(t("e2ee.unlock.rateLimited"));
      } else {
        setError(t("e2ee.newPin.failed"));
      }
    }
  };

  const conversation =
    safetyId === null ? null : friends.conversations.find((item) => item.id === safetyId) ?? null;
  const self = session.user;

  return (
    <>
      <PinSetupDialog
        open={kind === "setup"}
        pending={pending}
        error={error}
        onSubmit={setUp}
        onClose={close}
      />
      <UnlockDialog
        open={kind === "unlock"}
        pending={pending}
        error={error}
        dead={dead}
        onSubmit={unlock}
        onForgot={() => openDialog({ kind: "forgot" })}
        onClose={close}
      />
      <ForgotPinDialog
        open={kind === "forgot"}
        onContinue={() => openDialog({ kind: "newPin", mode: "reset" })}
        onClose={close}
      />
      <NewPinDialog
        open={kind === "newPin"}
        mode={dialog?.kind === "newPin" ? dialog.mode : "change"}
        pending={pending}
        passwordError={passwordError}
        error={error}
        onSubmit={(input) => void newPin(input)}
        onClose={close}
      />
      {conversation !== null && self !== null ? (
        <SafetyCodeDialog
          open={kind === "safety"}
          self={toPerson(self)}
          peer={toPerson(conversation.peer)}
          groups={code?.groups ?? null}
          error={error}
          mismatch={mismatch}
          onSame={() => {
            if (code === null) return;
            void port.setVerified(code.peerId, code.groups.join(" ")).finally(close);
          }}
          onDiffer={() => {
            if (code !== null) void port.clearVerified(code.peerId);
            setMismatch(true);
          }}
          onClose={close}
        />
      ) : null}
      <SecurityStepDialog {...security.dialog} username={self?.username ?? ""} t={t} />
    </>
  );
}
