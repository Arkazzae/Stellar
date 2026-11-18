import { Link } from "@stellarr/router";
import { useEffect, useId, useState, type FormEvent } from "react";

import { type Translator } from "@/i18n";
import { translateForDocument } from "@/i18n/defaultLocale";

import { AuthField, FormError } from "./AuthField";
import styles from "./AuthControls.module.css";
import { AuthFrame } from "./AuthFrame";

/** How long the page waits before it offers to send the link again. */
export const RESEND_AFTER_SECONDS = 60;

/**
 * Asks for a link to set a new password. The answer is the same for every
 * address, so the page never says whether one belongs to an account; after
 * sending it shows the address as typed, so a typo is easy to see, and waits
 * a minute before it offers to send again.
 */
export function ForgotPasswordScreen({
  onSubmit,
  instanceName = null,
  t = translateForDocument,
}: {
  onSubmit: (email: string) => Promise<void>;
  instanceName?: string | null;
  t?: Translator;
}) {
  const errorId = useId();
  const [email, setEmail] = useState("");
  const [pending, setPending] = useState(false);
  const [sentTo, setSentTo] = useState<string | null>(null);
  const [resendAt, setResendAt] = useState(0);
  const [now, setNow] = useState(() => Date.now());
  const [error, setError] = useState<string | null>(null);
  // Counted from the moment of sending, so a tab in the background, whose
  // timers the browser slows down, shows the right time when it comes back.
  const wait = Math.max(0, Math.ceil((resendAt - now) / 1000));

  useEffect(() => {
    if (resendAt <= Date.now()) return;
    const timer = window.setInterval(() => {
      const current = Date.now();
      setNow(current);
      if (current >= resendAt) window.clearInterval(timer);
    }, 1000);
    return () => window.clearInterval(timer);
  }, [resendAt]);

  async function send(address: string) {
    setPending(true);
    setError(null);
    try {
      await onSubmit(address);
      setSentTo(address);
      setNow(Date.now());
      setResendAt(Date.now() + RESEND_AFTER_SECONDS * 1000);
    } catch {
      setError(t("auth.forgot.failed"));
    } finally {
      setPending(false);
    }
  }

  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (pending) return;
    if (!email.trim()) {
      setError(t("auth.forgot.missing"));
      return;
    }
    void send(email.trim());
  }

  const back = (
    <div className={styles.foot}>
      <Link className={styles.link} href="/login">
        {t("auth.forgot.return")}
      </Link>
    </div>
  );

  if (sentTo !== null) {
    return (
      <AuthFrame
        scene="password"
        title={t("auth.forgot.sentTitle")}
        description={
          instanceName === null
            ? t("auth.forgot.sent", { address: sentTo })
            : t("auth.forgot.sentNamed", { address: sentTo, instance: instanceName })
        }
      >
        <div className={styles.stack}>
          <button
            type="button"
            className={`${styles.button} ${styles.secondary} ${styles.wide}`}
            onClick={() => void send(sentTo)}
            disabled={pending || wait > 0}
          >
            {wait > 0 ? t("auth.forgot.resendIn", { seconds: wait }) : t("auth.forgot.resend")}
          </button>
          {error ? <FormError>{error}</FormError> : null}
          <p className={styles.hint}>{t("auth.forgot.nothingCame")}</p>
          <div>
            <button type="button" className={`${styles.link} ${styles.small}`} onClick={() => setSentTo(null)}>
              {t("auth.forgot.otherAddress")}
            </button>
          </div>
        </div>
        {back}
      </AuthFrame>
    );
  }

  return (
    <AuthFrame scene="password" title={t("auth.forgot.title")} description={t("auth.forgot.description")}>
      <form className={styles.form} onSubmit={submit} noValidate>
        <AuthField
          label={t("auth.email")}
          name="email"
          type="email"
          autoComplete="email"
          value={email}
          onChange={(event) => setEmail(event.target.value)}
          disabled={pending}
          invalid={Boolean(error)}
          errorId={error ? errorId : undefined}
        />
        {error ? <FormError id={errorId}>{error}</FormError> : null}
        <button className={`${styles.button} ${styles.primary} ${styles.wide}`} type="submit" disabled={pending}>
          {pending ? t("auth.forgot.pending") : t("auth.forgot.submit")}
        </button>
      </form>
      {back}
    </AuthFrame>
  );
}
