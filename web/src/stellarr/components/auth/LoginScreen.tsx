import { Icon } from "@stellarr/icons";
import { Link } from "@stellarr/router";
import { useEffect, useId, useRef, useState, type FormEvent, type ReactNode } from "react";

import { AuthField, FormError, PasswordField } from "@/components/auth/AuthField";
import { AuthFrame } from "@/components/auth/AuthFrame";
import { InviteLinkForm } from "@/components/auth/InviteLinkForm";
import { type Translator } from "@/i18n";
import { translateForDocument } from "@/i18n/defaultLocale";

import styles from "./AuthControls.module.css";

export interface LoginScreenProps {
  onSubmit: (username: string, password: string) => void;
  pending?: boolean;
  error?: string | null;
  /** True while the existing session is being checked; the form is not offered yet. */
  checking?: boolean;
  t?: Translator;
  /** The instance's name for the description; a generic line until it is known. */
  instanceName?: string | null;
  /** Desktop disables Chromium's password-store prompt; web keeps normal autofill. */
  passwordAutoComplete?: "current-password" | "new-password";
  /** The native broker owns credentials, so CEF must not classify the field for its password UI. */
  nativeCredentialBroker?: boolean;
  /** Optional contextual action shown immediately above the login heading. */
  headingAction?: ReactNode;
  /** A link when `href` is given, a button when `onClick` is. */
  externalLogin?: {
    href?: string;
    label: string;
    onClick?: () => void;
    pending?: boolean;
  };
  externalError?: string | null;
  /**
   * "Keep me signed in", shown when given. The page holds the choice so the
   * identity provider's link carries it too.
   */
  keepSignedIn?: { checked: boolean; onChange: (checked: boolean) => void };
  /** The identity providers of the web sign-in page, as links, one per provider. */
  externalLogins?: { href: string; label: string; name: string }[];
  /** Signing in with a passkey alone, shown when given; the desktop app passes none. */
  passkeyLogin?: { label: string; onClick: () => void; pending?: boolean };
  /**
   * Opens the invite a pasted link names. Given, the foot says that accounts
   * here come from invites and offers a field for the link.
   */
  onInviteCode?: (code: string) => void;
}

/**
 * The password first, since most people sign in with it, and the other ways
 * in under it: a passkey and the instance's identity providers. Nobody signs
 * up here: an account comes from an invite, so the foot asks for the link.
 */
export function LoginScreen({
  onSubmit,
  pending = false,
  error = null,
  checking = false,
  t = translateForDocument,
  instanceName = null,
  passwordAutoComplete = "current-password",
  nativeCredentialBroker = false,
  headingAction,
  externalLogin,
  externalError = null,
  keepSignedIn,
  passkeyLogin,
  externalLogins = [],
  onInviteCode,
}: LoginScreenProps) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [localError, setLocalError] = useState<string | null>(null);
  const [inviteOpen, setInviteOpen] = useState(false);
  const usernameRef = useRef<HTMLInputElement>(null);
  const errorId = useId();
  const message = localError ?? error ?? externalError;
  const fieldState = { invalid: Boolean(message), errorId: message ? errorId : undefined };
  const alternatives = passkeyLogin !== undefined || externalLogin !== undefined || externalLogins.length > 0;

  useEffect(() => {
    if (checking) return;
    // A desktop user can type at once. On a phone the same focus would open the
    // software keyboard before the screen is read, covering the password field
    // and the recovery link.
    if (window.matchMedia("(hover: hover) and (pointer: fine)").matches) {
      usernameRef.current?.focus();
    }
  }, [checking]);

  useEffect(() => {
    if (error) usernameRef.current?.focus();
  }, [error]);

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (pending) return;
    if (!username.trim() || !password) {
      setLocalError(t("auth.login.missing"));
      usernameRef.current?.focus();
      return;
    }
    setLocalError(null);
    onSubmit(username.trim(), password);
  };

  return (
    <AuthFrame
      scene="login"
      title={t("auth.login.title")}
      description={
        instanceName === null
          ? t("auth.login.description")
          : t("auth.login.descriptionNamed", { instance: instanceName })
      }
      headingAction={headingAction}
    >
      {checking ? (
        <p className={styles.checking} aria-busy="true">
          {t("auth.login.checking")}
        </p>
      ) : (
        <>
          <form className={styles.form} onSubmit={submit} noValidate>
            <AuthField
              ref={usernameRef}
              label={t("auth.login.identifier")}
              name="username"
              type="text"
              autoComplete="username"
              autoCapitalize="none"
              spellCheck={false}
              value={username}
              onChange={(event) => setUsername(event.target.value)}
              disabled={pending}
              {...fieldState}
            />
            <PasswordField
              t={t}
              label={t("auth.password")}
              labelAction={
                <Link className={`${styles.link} ${styles.small}`} href="/forgot-password">
                  {t("auth.login.forgot")}
                </Link>
              }
              name="password"
              autoComplete={passwordAutoComplete}
              nativeCredentialBroker={nativeCredentialBroker}
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              disabled={pending}
              {...fieldState}
            />

            {keepSignedIn ? (
              <label className={styles.check}>
                <input
                  type="checkbox"
                  name="keepSignedIn"
                  checked={keepSignedIn.checked}
                  onChange={(event) => keepSignedIn.onChange(event.target.checked)}
                  disabled={pending}
                />
                <span>{t("auth.login.keepSignedIn")}</span>
              </label>
            ) : null}

            {message ? <FormError id={errorId}>{message}</FormError> : null}

            <button type="submit" className={`${styles.button} ${styles.primary} ${styles.wide}`} disabled={pending}>
              {pending ? t("auth.login.pending") : t("auth.login.submit")}
            </button>
          </form>

          {alternatives ? (
            <>
              <div className={styles.divider} aria-hidden="true">
                {t("auth.login.or")}
              </div>
              <div className={styles.alternatives}>
                {passkeyLogin ? (
                  <button
                    className={`${styles.button} ${styles.secondary}`}
                    type="button"
                    onClick={passkeyLogin.onClick}
                    disabled={pending || passkeyLogin.pending}
                    aria-label={t("auth.login.passkeyFull")}
                  >
                    <Icon name="key" size={20} />
                    <span>{passkeyLogin.label}</span>
                  </button>
                ) : null}
                {externalLogin === undefined ? null : externalLogin.onClick ? (
                  <button
                    className={`${styles.button} ${styles.secondary}`}
                    type="button"
                    onClick={externalLogin.onClick}
                    disabled={externalLogin.pending}
                  >
                    <Icon name="login" size={20} />
                    <span>{externalLogin.label}</span>
                  </button>
                ) : (
                  <a className={`${styles.button} ${styles.secondary}`} href={externalLogin.href}>
                    <Icon name="login" size={20} />
                    <span>{externalLogin.label}</span>
                  </a>
                )}
                {externalLogins.map((login) => (
                  <a
                    key={login.href}
                    className={`${styles.button} ${styles.secondary}`}
                    href={login.href}
                    aria-label={login.label}
                  >
                    <Icon name="login" size={20} />
                    <span>{login.name}</span>
                  </a>
                ))}
              </div>
            </>
          ) : null}

          {onInviteCode ? (
            <div className={styles.foot}>
              <p>
                {instanceName === null
                  ? t("auth.login.inviteOnly")
                  : t("auth.login.inviteOnlyNamed", { instance: instanceName })}
              </p>
              {inviteOpen ? (
                <InviteLinkForm t={t} onOpen={onInviteCode} autoFocus />
              ) : (
                <button className={styles.link} type="button" onClick={() => setInviteOpen(true)}>
                  {t("auth.login.haveInvite")}
                </button>
              )}
            </div>
          ) : null}
        </>
      )}
    </AuthFrame>
  );
}
