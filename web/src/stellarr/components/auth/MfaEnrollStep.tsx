import { Icon, type IconName } from "@stellarr/icons";
import { useEffect, useId, useRef, useState, type FormEvent, type ReactNode } from "react";

import { AuthField, FormError } from "@/components/auth/AuthField";
import { AuthFrame, type StepFrame } from "@/components/auth/AuthFrame";
import type { Translator } from "@/i18n";
import type { MfaChallenge, MfaEnrollment } from "@/ports";

import styles from "./AuthControls.module.css";
import { CodeCells } from "./CodeCells";
import steps from "./SignInSteps.module.css";

export type EnrollMode = "totp" | "email" | "passkey";

export interface MfaEnrollStepProps {
  challenge: MfaChallenge;
  t: Translator;
  mode: EnrollMode;
  /** The authenticator key being set up; null until the server has made one. */
  enrollment: MfaEnrollment | null;
  /** Seconds to wait before another email code, once the first one went out. */
  emailRetryAfter: number | null;
  pending: boolean;
  error: string | null;
  /** The QR image of `otpAuthUri`, drawn on this device. */
  qr?: ReactNode;
  onModeChange(mode: EnrollMode): void;
  onSendEmailCode(): void;
  onConfirm(input: { code: string; rememberDevice: boolean; name?: string }): void;
  onCancel(): void;
  /** Defaults to the sign-in page. */
  frame?: StepFrame;
  /** Added by choice from the account settings rather than required at sign-in. */
  voluntary?: boolean;
  cancelLabel?: string;
  /**
   * Creates a passkey; the browser asks where to keep it. Offered only when
   * this browser can use passkeys and the server can check them.
   */
  onCreatePasskey?(input: { rememberDevice: boolean; name?: string }): void;
  passkeysSupported?: boolean;
}

const ROW_ICON: Record<EnrollMode, IconName> = { passkey: "key", totp: "mobile", email: "mail" };

/** "ABCD EFGH IJKL", which is easier to copy by eye than one long run. */
const grouped = (secret: string) => secret.replace(/(.{4})/g, "$1 ").trim();

/**
 * Sets up the first method when a sign-in requires one, or a replacement after
 * an operator's recovery. The key is shown once and never kept by the client.
 *
 * The code is not sent on its sixth digit here, unlike at sign-in: the name
 * and the trust choice may still follow it.
 */
export function MfaEnrollStep({
  challenge,
  t,
  mode,
  enrollment,
  emailRetryAfter,
  pending,
  error,
  qr,
  onModeChange,
  onSendEmailCode,
  onConfirm,
  onCancel,
  frame: Frame = AuthFrame,
  voluntary = false,
  cancelLabel,
  onCreatePasskey,
  passkeysSupported = false,
}: MfaEnrollStepProps) {
  const [code, setCode] = useState("");
  const [name, setName] = useState("");
  const [remember, setRemember] = useState(false);
  const [copied, setCopied] = useState(false);
  const [localError, setLocalError] = useState<string | null>(null);
  const codeRef = useRef<HTMLInputElement>(null);
  const errorId = useId();
  const message = localError ?? error;
  const recovery = challenge.purpose === "recovery";
  const secret = enrollment?.type === "totp" ? enrollment.secret : undefined;
  const passkey = onCreatePasskey !== undefined && passkeysSupported && challenge.passkeyAvailable;

  useEffect(() => {
    if (error) codeRef.current?.focus();
  }, [error]);

  const copy = async () => {
    if (secret === undefined) return;
    try {
      await navigator.clipboard.writeText(secret);
      setCopied(true);
    } catch {
      // The key is selectable on screen, so a refused clipboard costs nothing.
    }
  };

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (pending) return;
    if (mode === "passkey") {
      setLocalError(null);
      onCreatePasskey?.({
        rememberDevice: remember,
        ...(name.trim() === "" ? {} : { name: name.trim() }),
      });
      return;
    }
    const typed = code.replace(/\s+/g, "");
    if (!typed) {
      setLocalError(t("auth.mfa.codeMissing"));
      codeRef.current?.focus();
      return;
    }
    setLocalError(null);
    onConfirm({
      code: typed,
      rememberDevice: remember,
      ...(name.trim() === "" ? {} : { name: name.trim() }),
    });
  };

  const switchTo = (next: EnrollMode) => {
    setCode("");
    setLocalError(null);
    setCopied(false);
    onModeChange(next);
  };

  const others: { mode: EnrollMode; name: string; detail: string }[] = [
    ...(passkey
      ? [{ mode: "passkey" as const, name: t("auth.mfa.wayPasskey"), detail: t("auth.mfa.wayPasskeyDetail") }]
      : []),
    { mode: "totp" as const, name: t("auth.mfa.wayTotp"), detail: t("auth.mfa.wayTotpDetail") },
    { mode: "email" as const, name: t("auth.mfa.wayEmail"), detail: t("auth.enroll.wayEmailDetail") },
  ].filter((row) => row.mode !== mode);

  const ready =
    mode === "passkey" || (mode === "email" ? emailRetryAfter !== null : secret !== undefined);

  return (
    <Frame
      title={
        recovery
          ? t("auth.enroll.recoveryTitle")
          : voluntary
            ? t("auth.enroll.voluntaryTitle")
            : t("auth.enroll.title")
      }
      description={
        recovery
          ? t("auth.enroll.recoveryDescription")
          : voluntary
            ? t("auth.enroll.voluntaryDescription")
            : t("auth.enroll.description")
      }
      scene="second-step"
    >
      <form className={styles.form} onSubmit={submit} noValidate>
        {mode === "totp" ? (
          secret === undefined ? (
            <p className={styles.checking} aria-busy="true">
              {t("auth.enroll.preparing")}
            </p>
          ) : (
            <>
              <p className={steps.note}>{t("auth.enroll.scan")}</p>
              {qr}
              <div className={steps.key}>
                <span className={steps.keyLabel} id={`${errorId}-key`}>
                  {t("auth.enroll.key")}
                </span>
                <div className={steps.keyRow}>
                  <code className={steps.keyValue} aria-labelledby={`${errorId}-key`}>
                    {grouped(secret)}
                  </code>
                  <button type="button" className={`${styles.button} ${styles.secondary}`} onClick={() => void copy()}>
                    {copied ? t("auth.enroll.copied") : t("auth.enroll.copy")}
                  </button>
                </div>
              </div>
            </>
          )
        ) : mode === "passkey" ? (
          <p className={steps.note}>{t("auth.enroll.passkeyDescription")}</p>
        ) : (
          <>
            <p className={steps.note}>{t("auth.enroll.emailDescription")}</p>
            <button
              type="button"
              className={`${styles.button} ${styles.secondary} ${styles.wide}`}
              onClick={onSendEmailCode}
              disabled={pending || (emailRetryAfter !== null && emailRetryAfter > 0)}
            >
              {emailRetryAfter === null
                ? t("auth.mfa.sendEmail")
                : emailRetryAfter > 0
                  ? t("auth.mfa.resendIn", { seconds: emailRetryAfter })
                  : t("auth.mfa.resendEmail")}
            </button>
          </>
        )}

        {ready ? (
          <>
            {voluntary ? (
              <AuthField
                label={t("auth.enroll.name")}
                hint={t("auth.enroll.nameHint")}
                name="method-name"
                type="text"
                autoComplete="off"
                maxLength={64}
                value={name}
                onChange={(event) => setName(event.target.value)}
                disabled={pending}
              />
            ) : null}
            {mode === "passkey" ? null : (
              <>
                <CodeCells
                  ref={codeRef}
                  label={t("auth.mfa.code")}
                  value={code}
                  onChange={setCode}
                  disabled={pending}
                  invalid={Boolean(message)}
                  errorId={message ? errorId : undefined}
                />
                <p className={styles.hint}>
                  {mode === "totp" ? t("auth.enroll.codeHint") : t("auth.mfa.emailSent")}
                </p>
              </>
            )}

            <label className={styles.check}>
              <input
                type="checkbox"
                checked={remember}
                onChange={(event) => setRemember(event.target.checked)}
                disabled={pending}
              />
              <span>{t("auth.mfa.remember", { days: challenge.trustedDeviceDays })}</span>
            </label>
          </>
        ) : null}

        {message ? <FormError id={errorId}>{message}</FormError> : null}

        {ready ? (
          <button type="submit" className={`${styles.button} ${styles.primary} ${styles.wide}`} disabled={pending}>
            {pending
              ? t("auth.mfa.pending")
              : mode === "passkey"
                ? t("auth.enroll.createPasskey")
                : t("auth.enroll.confirm")}
          </button>
        ) : null}
      </form>

      <section className={styles.stack} aria-labelledby={`${errorId}-others`}>
        <h2 className={styles.miniHead} id={`${errorId}-others`}>
          {t("auth.enroll.otherWays")}
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

      <div>
        <button type="button" className={styles.link} onClick={onCancel}>
          {cancelLabel ?? t("auth.mfa.cancel")}
        </button>
      </div>
    </Frame>
  );
}
