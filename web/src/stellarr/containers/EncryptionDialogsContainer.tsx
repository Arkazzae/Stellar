import { useEffect, useRef, useState } from "react";

import { SecurityStepDialog } from "@/components/account/panes/SecurityStepDialog";
import {
  LostRecoveryCodeDialog,
  RecoveryCodeDialog,
  SafetyCodeDialog,
  UnlockDialog,
} from "@/components/friends/EncryptionDialogs";
import { ApiError } from "@/lib/api/errors";
import { E2eeError } from "@/lib/e2ee/client";
import { pluralForm } from "@/lib/format/format";
import type { MessageKey } from "@/i18n";
import type { PreparedRecoveryCode, SafetyCodeView, UnlockResult } from "@/ports";
import { useEncryption, useFriends, useLanguage, usePorts, useSession } from "@/state/hooks";

import { FRESH_SECOND_STEP_REQUIRED } from "./freshSecondStep";
import { toPerson } from "./toPerson";
import { useAccountSecurity } from "./useAccountSecurity";

const WRONG_CODE: Record<"one" | "few" | "many", MessageKey> = {
  one: "e2ee.unlock.wrong.one",
  few: "e2ee.unlock.wrong.few",
  many: "e2ee.unlock.wrong.many",
};

export function EncryptionDialogsContainer() {
  const { encryption: port } = usePorts();
  const { dialog, closeDialog, openDialog, runSwitch, status } = useEncryption();
  const friends = useFriends();
  const session = useSession();
  const { t, locale } = useLanguage();
  const security = useAccountSecurity(dialog?.kind === "recoveryCode", { withSessions: false });

  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [passwordError, setPasswordError] = useState<string | null>(null);
  const [dead, setDead] = useState(false);
  const [comparison, setCode] = useState<{ view: SafetyCodeView; id: string; revision: number } | null>(null);
  const [mismatch, setMismatch] = useState(false);
  /** Reuse the same encrypted backup when re-authentication needs a second attempt. */
  const prepared = useRef<{ key: string; value: PreparedRecoveryCode } | null>(null);

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
  const conversation =
    safetyId === null ? null : friends.conversations.find((item) => item.id === safetyId) ?? null;
  const expectedPeer = conversation?.peer.id;
  const code = comparison?.id === safetyId && comparison.revision === status.revision ? comparison.view : null;
  useEffect(() => {
    if (safetyId === null) return;
    let current = true;
    port.safetyCode(safetyId).then(
      (view) => {
        if (!current) return;
        if (view === null || view.peerId !== expectedPeer) setError(t("e2ee.safety.failed"));
        else {
          setError(null);
          setCode({ view, id: safetyId, revision: status.revision });
        }
      },
      () => current && setError(t("e2ee.safety.failed")),
    );
    return () => {
      current = false;
    };
  }, [port, safetyId, expectedPeer, status.revision, t]);

  const close = () => {
    prepared.current = null;
    closeDialog();
  };

  const setUp = (code: string) => {
    if (dialog?.kind !== "setup") return;
    const then = dialog.then;
    setPending(true);
    setError(null);
    port.setUp(code).then(
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
      case "invalid-code":
        return t("e2ee.recovery.invalid");
      case "wrong-code":
        return t(WRONG_CODE[pluralForm(locale, result.remainingAttempts)], { count: result.remainingAttempts });
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

  const unlock = (input: string) => {
    setPending(true);
    setError(null);
    port.unlock(input).then(
      (result) => {
        setPending(false);
        if (result.kind === "unlocked") {
          if (status.migrationRequired) openDialog({ kind: "recoveryCode", mode: "change" });
          else close();
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

  const saveRecoveryCode = async (code: string, password: string) => {
    if (dialog?.kind !== "recoveryCode") return;
    const { mode } = dialog;
    setPending(true);
    setError(null);
    setPasswordError(null);
    try {
      const key = `${mode}:${code}`;
      if (prepared.current?.key !== key) {
        prepared.current = {
          key,
          value: mode === "change" ? await port.prepareRecoveryCode(code) : await port.prepareReset(code),
        };
      }
      const value = prepared.current.value;
      const attempt = () =>
        mode === "change"
          ? port.changeRecoveryCode(value, password)
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
      } else if ((cause instanceof E2eeError && cause.reason === "key-changed") || (cause instanceof ApiError && cause.code === "account-key-changed")) {
        setError(t("e2ee.newPin.keyChanged"));
      } else if (cause instanceof ApiError && cause.kind === "rate-limited") {
        setError(t("e2ee.unlock.rateLimited"));
      } else {
        setError(t("e2ee.newPin.failed"));
      }
    }
  };

  const self = session.user;

  return (
    <>
      {(kind === "setup" || dialog?.kind === "recoveryCode") && (
        <RecoveryCodeDialog key={dialog?.kind === "recoveryCode" ? dialog.mode : "setup"}
          mode={dialog?.kind === "recoveryCode" ? dialog.mode : "setup"}
          migration={status.migrationRequired === true && dialog?.kind === "recoveryCode" && dialog.mode === "change"}
          pending={pending} error={error} passwordError={passwordError}
          withPassword={kind === "recoveryCode" && security.sections.security?.hasPassword !== false}
          onSubmit={(code, password) => kind === "setup" ? setUp(code) : void saveRecoveryCode(code, password)}
          onClose={close} />
      )}
      {kind === "unlock" && <UnlockDialog
        legacy={status.migrationRequired === true}
        open
        pending={pending}
        error={error}
        dead={dead}
        onSubmit={unlock}
        onForgot={() => openDialog({ kind: "forgot" })}
        onClose={close}
      />}
      <LostRecoveryCodeDialog
        open={kind === "forgot"}
        onContinue={() => openDialog({ kind: "recoveryCode", mode: "reset" })}
        onClose={close}
      />
      {conversation !== null && self !== null ? (
        <SafetyCodeDialog
          open={kind === "safety"}
          self={toPerson(self)}
          peer={toPerson(conversation.peer)}
          groups={code?.peerId === expectedPeer && code !== null ? code.groups : null}
          error={error}
          mismatch={mismatch}
          onSame={() => {
            if (code === null || code.peerId !== expectedPeer) return;
            void port.setVerified(code.peerId, code.groups.join(" ")).finally(close);
          }}
          onDiffer={() => {
            if (code !== null && code.peerId === expectedPeer) void port.setVerified(code.peerId, null);
            setMismatch(true);
          }}
          onClose={close}
        />
      ) : null}
      <SecurityStepDialog {...security.dialog} username={self?.username ?? ""} t={t} />
    </>
  );
}
