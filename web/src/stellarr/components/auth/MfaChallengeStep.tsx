import { Icon, type IconName } from "@stellarr/icons";
import { useEffect, useId, useRef, useState, type FormEvent } from "react";

import { AuthField, FormError } from "@/components/auth/AuthField";
import { AuthFrame, type StepFrame } from "@/components/auth/AuthFrame";
import type { Translator } from "@/i18n";
import type { MfaChallenge, MfaMethod } from "@/ports";

import styles from "./AuthControls.module.css";
import { CodeCells } from "./CodeCells";
import { Chip, Face } from "./Faces";
import steps from "./SignInSteps.module.css";

type Mode = "passkey" | "totp" | "email" | "recovery";

export interface MfaChallengeStepProps {
  challenge: MfaChallenge;
  t: Translator;
  pending: boolean;
  error: string | null;
  onVerify(input: { methodId?: string; code: string; rememberDevice: boolean }): void;
  onUseTrustedDevice(): void;
  /** Resolves with the seconds until another code may be sent, or null when sending failed. */
  onSendEmailCode(methodId: string): Promise<number | null>;
  onCancel(): void;
  /** Defaults to the sign-in page. */
  frame?: StepFrame;
  /**
   * Offers to trust this device. Off for a proof before a security change,
   * which a trusted device cannot give.
   */
  offerTrust?: boolean;
  cancelLabel?: string;
  /**
   * Names the account the step belongs to, with "Not me" to go back to the
   * password. Only a sign-in shows it; a proof inside the settings does not.
   */
  showAccount?: boolean;
  /**
   * Answers with a passkey; the browser asks for it. Offered only when this
   * browser can use passkeys and the account has one.
   */
  onUsePasskey?(rememberDevice: boolean): void;
  passkeysSupported?: boolean;
}

const confirmed = (methods: readonly MfaMethod[], type: MfaMethod["type"]) =>
  methods.filter((method) => method.type === type && !method.pending);

const ROW_ICON: Record<Mode, IconName> = {
  passkey: "key",
  totp: "mobile",
  email: "mail",
  recovery: "list",
};

/**
 * The second step of a sign-in. It opens on a passkey when the account has
 * one and the browser can use it, else on the authenticator app, which needs
 * no round trip, and lists the email code and a recovery code as the other
 * ways in, each with what it needs.
 *
 * A six-digit code goes to the server by itself once the sixth digit is in,
 * so the choice to trust this device sits above the cells, where it can be
 * made before.
 */
export function MfaChallengeStep({
  challenge,
  t,
  pending,
  error,
  onVerify,
  onUseTrustedDevice,
  onSendEmailCode,
  onCancel,
  frame: Frame = AuthFrame,
  offerTrust = true,
  cancelLabel,
  showAccount = false,
  onUsePasskey,
  passkeysSupported = false,
}: MfaChallengeStepProps) {
  const apps = confirmed(challenge.methods, "totp");
  const emails = confirmed(challenge.methods, "email");
  const email = emails[0];
  const passkey =
    onUsePasskey !== undefined && passkeysSupported && confirmed(challenge.methods, "passkey").length > 0;
  const [mode, setMode] = useState<Mode>(
    passkey ? "passkey" : apps.length > 0 ? "totp" : email ? "email" : "recovery",
  );
  const [appId, setAppId] = useState(apps[0]?.id);
  const [code, setCode] = useState("");
  const [remember, setRemember] = useState(false);
  const [localError, setLocalError] = useState<string | null>(null);
  const [sent, setSent] = useState(false);
  const [wait, setWait] = useState(0);
  const codeRef = useRef<HTMLInputElement>(null);
  const errorId = useId();
  const message = localError ?? error;
  const digits = mode === "totp" || mode === "email";
  const account = showAccount ? challenge.account : undefined;

  useEffect(() => {
    if (wait <= 0) return;
    const timer = window.setTimeout(() => setWait((seconds) => seconds - 1), 1000);
    return () => window.clearTimeout(timer);
  }, [wait]);

  // A refused code is cleared, so the next one can be typed without deleting
  // six digits first.
  const [refusal, setRefusal] = useState(error);
  if (error !== refusal) {
    setRefusal(error);
    if (error) setCode("");
  }

  useEffect(() => {
    if (error) codeRef.current?.focus();
  }, [error]);

  const switchTo = (next: Mode) => {
    setMode(next);
    setCode("");
    setLocalError(null);
    window.setTimeout(() => codeRef.current?.focus(), 0);
  };

  const send = async () => {
    if (email === undefined || wait > 0) return;
    const seconds = await onSendEmailCode(email.id);
    if (seconds === null) return;
    setSent(true);
    setWait(seconds);
    codeRef.current?.focus();
  };

  const verify = (typed: string) => {
    if (pending) return;
    if (!typed) {
      setLocalError(t("auth.mfa.codeMissing"));
      codeRef.current?.focus();
      return;
    }
    setLocalError(null);
    onVerify({
      ...(mode === "totp" ? { methodId: appId } : mode === "email" ? { methodId: email?.id } : {}),
      code: typed,
      rememberDevice: offerTrust && remember,
    });
  };

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (pending) return;
    if (mode === "passkey") {
      setLocalError(null);
      onUsePasskey?.(offerTrust && remember);
      return;
    }
    verify(mode === "recovery" ? code.trim() : code);
  };

  const title = {
    passkey: t("auth.mfa.passkeyTitle"),
    totp: t("auth.mfa.totpTitle"),
    email: t("auth.mfa.emailTitle"),
    recovery: t("auth.mfa.recoveryTitle"),
  }[mode];
  const description = {
    passkey: t("auth.mfa.passkeyDescription"),
    totp: t("auth.mfa.totpDescription"),
    email: t("auth.mfa.emailDescription", { address: email?.emailAddress ?? "" }),
    recovery: t("auth.mfa.recoveryDescription"),
  }[mode];

  const others: { mode: Mode; name: string; detail: string }[] = [
    ...(passkey
      ? [{ mode: "passkey" as const, name: t("auth.mfa.wayPasskey"), detail: t("auth.mfa.wayPasskeyDetail") }]
      : []),
    ...(apps.length > 0
      ? [{ mode: "totp" as const, name: t("auth.mfa.wayTotp"), detail: t("auth.mfa.wayTotpDetail") }]
      : []),
    ...(email !== undefined
      ? [
          {
            mode: "email" as const,
            name: t("auth.mfa.wayEmail"),
            detail: t("auth.mfa.wayEmailDetail", { address: email.emailAddress ?? "" }),
          },
        ]
      : []),
    { mode: "recovery" as const, name: t("auth.mfa.wayRecovery"), detail: t("auth.mfa.wayRecoveryDetail") },
  ].filter((row) => row.mode !== mode);

  return (
    <Frame title={title} description={description} scene="second-step">
      {account === undefined ? null : (
        <Chip
          face={<Face name={account.displayName} color={account.avatarColor} imageUrl={account.avatarUrl} />}
          name={account.displayName}
          detail={<span className={styles.mono}>@{account.username}</span>}
          end={
            <button type="button" className={styles.miniButton} onClick={onCancel} disabled={pending}>
              {t("auth.mfa.notMe")}
            </button>
          }
        />
      )}

      {offerTrust && challenge.trustedDeviceAvailable ? (
        <>
          <button
            type="button"
            className={`${styles.button} ${styles.secondary} ${styles.wide}`}
            onClick={onUseTrustedDevice}
            disabled={pending}
          >
            {t("auth.mfa.useTrustedDevice")}
          </button>
          <div className={styles.divider} aria-hidden="true">
            {t("auth.mfa.orCode")}
          </div>
        </>
      ) : null}

      <form className={styles.form} onSubmit={submit} noValidate>
        {mode === "totp" && apps.length > 1 ? (
          <div className={styles.field}>
            <label className={styles.label} htmlFor={`${errorId}-app`}>
              {t("auth.mfa.app")}
            </label>
            <select
              id={`${errorId}-app`}
              className={steps.select}
              value={appId}
              onChange={(event) => setAppId(event.target.value)}
              disabled={pending}
            >
              {apps.map((app) => (
                <option key={app.id} value={app.id}>
                  {app.name}
                </option>
              ))}
            </select>
          </div>
        ) : null}

        {mode === "email" ? (
          <button
            type="button"
            className={`${styles.button} ${styles.secondary} ${styles.wide}`}
            onClick={() => void send()}
            disabled={pending || wait > 0}
          >
            {wait > 0
              ? t("auth.mfa.resendIn", { seconds: wait })
              : sent
                ? t("auth.mfa.resendEmail")
                : t("auth.mfa.sendEmail")}
          </button>
        ) : null}

        {offerTrust ? (
          <label className={styles.check}>
            <input
              type="checkbox"
              checked={remember}
              onChange={(event) => setRemember(event.target.checked)}
              disabled={pending}
            />
            <span>{t("auth.mfa.remember", { days: challenge.trustedDeviceDays })}</span>
          </label>
        ) : null}

        {digits ? (
          <>
            <CodeCells
              ref={codeRef}
              label={t("auth.mfa.code")}
              value={code}
              onChange={setCode}
              onComplete={verify}
              disabled={pending}
              invalid={Boolean(message)}
              errorId={message ? errorId : undefined}
              autoFocus={mode === "totp"}
            />
            {mode === "email" && sent ? <p className={styles.hint}>{t("auth.mfa.emailSent")}</p> : null}
          </>
        ) : mode === "recovery" ? (
          <AuthField
            ref={codeRef}
            label={t("auth.mfa.recoveryCode")}
            name="code"
            type="text"
            autoComplete="off"
            autoCapitalize="characters"
            spellCheck={false}
            maxLength={64}
            className={styles.mono}
            value={code}
            onChange={(event) => setCode(event.target.value)}
            disabled={pending}
            autoFocus
            invalid={Boolean(message)}
            errorId={message ? errorId : undefined}
          />
        ) : null}

        {message ? <FormError id={errorId}>{message}</FormError> : null}

        <button type="submit" className={`${styles.button} ${styles.primary} ${styles.wide}`} disabled={pending}>
          {pending
            ? t("auth.mfa.pending")
            : mode === "passkey"
              ? t("auth.mfa.usePasskey")
              : t("auth.mfa.submit")}
        </button>
      </form>

      <section className={styles.stack} aria-labelledby={`${errorId}-others`}>
        <h2 className={styles.miniHead} id={`${errorId}-others`}>
          {t("auth.mfa.otherWays")}
        </h2>
        <div className={styles.rows}>
          {others.map((row) => (
            <button
              key={row.mode}
              type="button"
              className={styles.row}
              onClick={() => switchTo(row.mode)}
              disabled={pending}
            >
              <Icon name={ROW_ICON[row.mode]} size={20} />
              <span>
                <strong>{row.name}</strong>
                <small>{row.detail}</small>
              </span>
              <Icon name="caret" size={16} />
            </button>
          ))}
        </div>
      </section>

      {account === undefined ? (
        <div>
          <button type="button" className={styles.link} onClick={onCancel}>
            {cancelLabel ?? t("auth.mfa.cancel")}
          </button>
        </div>
      ) : null}
    </Frame>
  );
}
