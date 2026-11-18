import { Link } from "@stellarr/router";
import { useId, useState, type FormEvent } from "react";

import { type Translator } from "@/i18n";
import { translateForDocument } from "@/i18n/defaultLocale";
import { MIN_PASSWORD_LENGTH } from "@/lib/auth/passwordStrength";

import { FormError, PasswordField } from "./AuthField";
import styles from "./AuthControls.module.css";
import { AuthFrame } from "./AuthFrame";
import { StrengthMeter } from "./StrengthMeter";

/**
 * Sets the new password from the mailed link. "Show" lets a person check
 * what they typed, so there is no second field to type it into again.
 * Saving it ends every other session of the account, and the page says so.
 */
export function ResetPasswordScreen({
  token,
  onSubmit,
  t = translateForDocument,
}: {
  token: string;
  onSubmit: (token: string, password: string) => Promise<void>;
  t?: Translator;
}) {
  const errorId = useId();
  const [password, setPassword] = useState("");
  const [pending, setPending] = useState(false);
  const [changed, setChanged] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (pending) return;
    if (password.length < MIN_PASSWORD_LENGTH) {
      setError(t("auth.password.length"));
      return;
    }
    setPending(true);
    setError(null);
    try {
      await onSubmit(token, password);
      setChanged(true);
      setPassword("");
    } catch {
      setError(t("auth.reset.invalid"));
    } finally {
      setPending(false);
    }
  }

  if (!token) {
    return (
      <AuthFrame scene="password" title={t("auth.reset.missingTitle")} description={t("auth.reset.missingDescription")}>
        <Link className={`${styles.button} ${styles.primary} ${styles.wide}`} href="/forgot-password">
          {t("auth.reset.newLink")}
        </Link>
      </AuthFrame>
    );
  }

  if (changed) {
    return (
      <AuthFrame scene="password" title={t("auth.reset.successTitle")} description={t("auth.reset.success")}>
        <Link className={`${styles.button} ${styles.primary} ${styles.wide}`} href="/login">
          {t("auth.reset.goLogin")}
        </Link>
      </AuthFrame>
    );
  }

  return (
    <AuthFrame scene="password" title={t("auth.reset.title")} description={t("auth.reset.description")}>
      <form className={styles.form} onSubmit={(event) => void submit(event)} noValidate>
        <PasswordField
          t={t}
          label={t("auth.reset.newPassword")}
          name="password"
          autoComplete="new-password"
          value={password}
          onChange={(event) => setPassword(event.target.value)}
          disabled={pending}
          invalid={Boolean(error)}
          errorId={error ? errorId : undefined}
        />
        <StrengthMeter password={password} username="" t={t} />
        {error ? <FormError id={errorId}>{error}</FormError> : null}
        <button className={`${styles.button} ${styles.primary} ${styles.wide}`} type="submit" disabled={pending}>
          {pending ? t("auth.reset.pending") : t("auth.reset.submit")}
        </button>
      </form>
    </AuthFrame>
  );
}
