import { useId, useRef, useState, type FormEvent } from "react";

import { AuthField, FormError } from "@/components/auth/AuthField";
import { AuthFrame } from "@/components/auth/AuthFrame";
import type { Translator } from "@/i18n";

import styles from "./AuthControls.module.css";

export interface RecoveryRedeemStepProps {
  t: Translator;
  pending: boolean;
  error: string | null;
  onSubmit(credential: string): void;
  onBack(): void;
}

/**
 * After an operator reset, a password alone no longer signs in. The one-time
 * credential the operator passed on starts the setup of new security, and
 * nothing else.
 */
export function RecoveryRedeemStep({ t, pending, error, onSubmit, onBack }: RecoveryRedeemStepProps) {
  const [credential, setCredential] = useState("");
  const [localError, setLocalError] = useState<string | null>(null);
  const fieldRef = useRef<HTMLInputElement>(null);
  const errorId = useId();
  const message = localError ?? error;

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (pending) return;
    const typed = credential.trim();
    if (!typed) {
      setLocalError(t("auth.mfa.codeMissing"));
      fieldRef.current?.focus();
      return;
    }
    setLocalError(null);
    onSubmit(typed);
  };

  return (
    <AuthFrame title={t("auth.recover.title")} description={t("auth.recover.description")} scene="second-step">
      <form className={styles.form} onSubmit={submit} noValidate>
        <AuthField
          ref={fieldRef}
          label={t("auth.recover.credential")}
          name="recovery-credential"
          type="text"
          autoComplete="off"
          autoCapitalize="none"
          spellCheck={false}
          className={styles.mono}
          value={credential}
          onChange={(event) => setCredential(event.target.value)}
          disabled={pending}
          invalid={Boolean(message)}
          errorId={message ? errorId : undefined}
        />
        {message ? <FormError id={errorId}>{message}</FormError> : null}
        <button type="submit" className={`${styles.button} ${styles.primary} ${styles.wide}`} disabled={pending}>
          {pending ? t("auth.mfa.pending") : t("auth.recover.submit")}
        </button>
      </form>
      <div>
        <button type="button" className={styles.link} onClick={onBack}>
          {t("auth.recover.back")}
        </button>
      </div>
    </AuthFrame>
  );
}
