import { useId, useRef, useState, type FormEvent } from "react";

import { FormError, PasswordField } from "@/components/auth/AuthField";
import { AuthFrame, type StepFrame } from "@/components/auth/AuthFrame";
import type { Translator } from "@/i18n";

import styles from "./AuthControls.module.css";

export interface PasswordConfirmStepProps {
  t: Translator;
  pending: boolean;
  error: string | null;
  onSubmit(password: string): void;
  /** Defaults to the sign-in page. */
  frame?: StepFrame;
  /** A way out, offered on the page; a dialog has its own close button. */
  onBack?(): void;
}

/** The password, asked again before the first second-factor method is set up. */
export function PasswordConfirmStep({
  t,
  pending,
  error,
  onSubmit,
  frame: Frame = AuthFrame,
  onBack,
}: PasswordConfirmStepProps) {
  const [password, setPassword] = useState("");
  const fieldRef = useRef<HTMLInputElement>(null);
  const errorId = useId();

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (pending || !password) {
      fieldRef.current?.focus();
      return;
    }
    onSubmit(password);
  };

  return (
    <Frame title={t("security.password.title")} description={t("security.password.description")} scene="second-step">
      <form className={styles.form} onSubmit={submit} noValidate>
        <PasswordField
          ref={fieldRef}
          t={t}
          label={t("auth.password")}
          name="password"
          autoComplete="current-password"
          value={password}
          onChange={(event) => setPassword(event.target.value)}
          disabled={pending}
          autoFocus
          invalid={Boolean(error)}
          errorId={error ? errorId : undefined}
        />
        {error ? <FormError id={errorId}>{error}</FormError> : null}
        <button type="submit" className={`${styles.button} ${styles.primary} ${styles.wide}`} disabled={pending}>
          {pending ? t("auth.mfa.pending") : t("security.password.submit")}
        </button>
      </form>
      {onBack ? (
        <div>
          <button type="button" className={styles.link} onClick={onBack}>
            {t("common.cancel")}
          </button>
        </div>
      ) : null}
    </Frame>
  );
}
