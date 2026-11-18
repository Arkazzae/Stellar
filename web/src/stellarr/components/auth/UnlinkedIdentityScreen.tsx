import { Icon, type IconName } from "@stellarr/icons";
import { useEffect, useId, useRef, useState, type FormEvent, type ReactNode } from "react";

import { avatarColor } from "@/design/identity";
import type { Translator } from "@/i18n";
import type { PendingIdentity } from "@/ports";

import { AuthField, FormError, PasswordField } from "./AuthField";
import styles from "./AuthControls.module.css";
import { AuthFrame } from "./AuthFrame";
import { Chip, Face } from "./Faces";
import { InviteLinkForm } from "./InviteLinkForm";

type Choice = "link" | "invite";

/** One way on: a row that names it and opens its form under it. */
function ChoiceRow({
  open,
  icon,
  title,
  detail,
  onToggle,
  disabled,
  children,
}: {
  open: boolean;
  icon: IconName;
  title: string;
  detail: string;
  onToggle(): void;
  disabled: boolean;
  children?: ReactNode;
}) {
  const panelId = useId();
  return (
    <div className={styles.choice} data-open={open || undefined}>
      <button
        type="button"
        className={styles.row}
        aria-expanded={children === undefined ? undefined : open}
        aria-controls={children === undefined ? undefined : panelId}
        onClick={onToggle}
        disabled={disabled}
      >
        <Icon name={icon} size={20} />
        <span>
          <strong>{title}</strong>
          <small>{detail}</small>
        </span>
        <Icon name="caret" size={16} />
      </button>
      {open && children !== undefined ? (
        <div id={panelId} className={styles.expand}>
          {children}
        </div>
      ) : null}
    </div>
  );
}

export interface UnlinkedIdentityScreenProps {
  t: Translator;
  identity: PendingIdentity;
  instanceName: string | null;
  /** An invite this tab remembered from an invite page, so the invite row goes straight on. */
  inviteRemembered: boolean;
  pending: boolean;
  error: string | null;
  /** Signs in with the password and asks to link the identity once that is done. */
  onLink(username: string, password: string): void;
  /** Goes on to the account made through the provider, with the invite in `code` or the one remembered. */
  onInvite(code: string | null): void;
  /** Forgets the identity and goes back to the sign-in page. */
  onBack(): void;
}

/**
 * The page a provider's sign-in lands on when no account here is linked to
 * the identity it confirmed. It shows whose identity it is and offers the
 * two ways on: an account here, signed in with its password and then linked,
 * or an invite, which makes an account through the provider when the owner
 * lets it. It never says whether the provider's address belongs to an
 * account here: that would tell anyone with a provider account who is on
 * the instance.
 */
export function UnlinkedIdentityScreen({
  t,
  identity,
  instanceName,
  inviteRemembered,
  pending,
  error,
  onLink,
  onInvite,
  onBack,
}: UnlinkedIdentityScreenProps) {
  const provider = identity.provider.displayName;
  const [open, setOpen] = useState<Choice | null>(null);
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [missing, setMissing] = useState(false);
  const usernameRef = useRef<HTMLInputElement>(null);
  const errorId = useId();
  const message = missing ? t("auth.login.missing") : error;
  const fieldState = { invalid: Boolean(message), errorId: message ? errorId : undefined };
  const name = identity.displayName ?? identity.email ?? provider;

  useEffect(() => {
    if (open === "link") usernameRef.current?.focus();
  }, [open]);

  useEffect(() => {
    if (error) usernameRef.current?.focus();
  }, [error]);

  const toggle = (choice: Choice) => setOpen((current) => (current === choice ? null : choice));

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (pending) return;
    if (!username.trim() || !password) {
      setMissing(true);
      usernameRef.current?.focus();
      return;
    }
    setMissing(false);
    onLink(username.trim(), password);
  };

  return (
    <AuthFrame
      scene="unlinked"
      title={t("auth.unlinked.title", { provider })}
      description={
        instanceName === null
          ? t("auth.unlinked.description", { provider })
          : t("auth.unlinked.descriptionNamed", { provider, instance: instanceName })
      }
    >
      <Chip
        face={<Face name={name} color={avatarColor(identity.email ?? name)} />}
        name={name}
        detail={
          identity.email === null ? (
            provider
          ) : (
            <>
              <span className={styles.mono}>{identity.email}</span> · {provider}
            </>
          )
        }
      />

      <div className={styles.rows}>
        <ChoiceRow
          open={open === "link"}
          icon="user"
          title={
            instanceName === null
              ? t("auth.unlinked.haveAccount")
              : t("auth.unlinked.haveAccountNamed", { instance: instanceName })
          }
          detail={t("auth.unlinked.haveAccountDetail", { provider })}
          onToggle={() => toggle("link")}
          disabled={false}
        >
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
              name="password"
              autoComplete="current-password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              disabled={pending}
              {...fieldState}
            />
            {message ? <FormError id={errorId}>{message}</FormError> : null}
            <button type="submit" className={`${styles.button} ${styles.primary} ${styles.wide}`} disabled={pending}>
              {pending ? t("auth.login.pending") : t("auth.unlinked.linkSubmit")}
            </button>
          </form>
        </ChoiceRow>
        {identity.signUp ? (
          <ChoiceRow
            open={open === "invite"}
            icon="link"
            title={t("auth.unlinked.haveInvite")}
            detail={t("auth.unlinked.haveInviteDetail", { provider })}
            // An invite opened before the provider was chosen needs no pasting.
            onToggle={() => (inviteRemembered ? onInvite(null) : toggle("invite"))}
            disabled={pending}
          >
            {inviteRemembered ? undefined : <InviteLinkForm t={t} onOpen={onInvite} autoFocus />}
          </ChoiceRow>
        ) : null}
      </div>

      <div className={styles.foot}>
        <p>{t("auth.unlinked.noInvite")}</p>
        <button className={styles.link} type="button" onClick={onBack}>
          {t("auth.invite.backToLogin")}
        </button>
      </div>
    </AuthFrame>
  );
}
