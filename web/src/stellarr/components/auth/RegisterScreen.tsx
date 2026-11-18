import { Icon } from "@stellarr/icons";
import { Link } from "@stellarr/router";
import { useEffect, useId, useRef, useState, type FormEvent } from "react";

import { type Translator } from "@/i18n";
import type { PendingIdentity } from "@/ports";

import {
  AccountFields,
  accountProblem,
  focusField,
  useAccountValues,
  type AccountValues,
  type FormIssue,
} from "./AccountFields";
import { FormError } from "./AuthField";
import styles from "./AuthControls.module.css";
import { AuthFrame } from "./AuthFrame";
import { InviteLinkForm } from "./InviteLinkForm";

/**
 * Creating an account with an invite. The account joins the invite's server
 * as it is created, so the description says where it leads and who asked.
 * The providers that may create accounts are offered under the form, each
 * as the way to one with no password.
 */
export function RegisterScreen({
  onSubmit,
  invite,
  providers = [],
  t,
}: {
  /** Resolves to null once the account exists, or to what the server refused. */
  onSubmit: (values: AccountValues) => Promise<FormIssue | null>;
  /** The invite's server and who sent it, once the preview is here. */
  invite: { server: string; inviter: string } | null;
  /** The address that starts each provider's sign-in, which comes back to the account made through it. */
  providers?: { name: string; href: string }[];
  t: Translator;
}) {
  const errorId = useId();
  const form = useRef<HTMLFormElement>(null);
  const { values, change } = useAccountValues();
  const [pending, setPending] = useState(false);
  const [issue, setIssue] = useState<FormIssue | null>(null);

  // After the render that shows the problem, once the fields are enabled
  // again. On a phone the field may be screens above the button, so the
  // focus is also what scrolls to it.
  useEffect(() => {
    if (issue?.field) focusField(form.current, issue.field);
  }, [issue]);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (pending) return;

    const problem = accountProblem(values, t);
    if (problem) {
      setIssue(problem);
      return;
    }

    setPending(true);
    setIssue(null);
    const refused = await onSubmit({
      displayName: values.displayName.trim(),
      username: values.username.trim(),
      email: values.email.trim(),
      password: values.password,
    });
    setPending(false);
    setIssue(refused);
  }

  return (
    <AuthFrame
      scene="register"
      title={t("auth.register.title")}
      description={
        invite === null
          ? t("auth.register.description")
          : t("auth.register.descriptionInvite", { server: invite.server, inviter: invite.inviter })
      }
    >
      <form ref={form} className={styles.form} onSubmit={(event) => void submit(event)} noValidate>
        <AccountFields
          t={t}
          values={values}
          onChange={change}
          issue={issue}
          errorId={errorId}
          disabled={pending}
          previewTag={t("auth.register.previewTag")}
        />
        {issue ? <FormError id={errorId}>{issue.message}</FormError> : null}
        <button className={`${styles.button} ${styles.primary} ${styles.wide}`} type="submit" disabled={pending}>
          {pending ? t("auth.register.pending") : t("auth.register.submit")}
        </button>
      </form>
      {providers.length === 0 ? null : (
        <>
          <div className={styles.divider} aria-hidden="true">
            {t("auth.login.or")}
          </div>
          <div className={styles.stack}>
            {providers.map((provider) => (
              <a key={provider.href} className={`${styles.button} ${styles.secondary} ${styles.wide}`} href={provider.href}>
                <Icon name="login" size={20} />
                <span>{t("auth.register.withProvider", { provider: provider.name })}</span>
              </a>
            ))}
          </div>
        </>
      )}
      <div className={styles.foot}>
        <p>{t("auth.register.haveAccount")}</p>
        <Link className={styles.link} href="/login">
          {t("auth.register.logIn")}
        </Link>
      </div>
    </AuthFrame>
  );
}

/**
 * Creating an account from the identity a provider confirmed, with an
 * invite. The form is filled with what the provider said, which the person
 * may change; there is no password, as the account signs in through the
 * provider. Whether the address is verified is the server's to decide: only
 * the one the provider vouched for counts.
 */
export function ProviderRegisterScreen({
  identity,
  invite,
  onSubmit,
  t,
}: {
  identity: PendingIdentity;
  /** The invite's server and who sent it, once the preview is here. */
  invite: { server: string; inviter: string } | null;
  /** Resolves to null once the account exists, or to what the server refused. */
  onSubmit: (values: Omit<AccountValues, "password">) => Promise<FormIssue | null>;
  t: Translator;
}) {
  const provider = identity.provider.displayName;
  const errorId = useId();
  const form = useRef<HTMLFormElement>(null);
  const { values, change } = useAccountValues({
    displayName: identity.displayName ?? "",
    username: identity.username,
    email: identity.email ?? "",
  });
  const [pending, setPending] = useState(false);
  const [issue, setIssue] = useState<FormIssue | null>(null);

  useEffect(() => {
    if (issue?.field) focusField(form.current, issue.field);
  }, [issue]);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (pending) return;
    const problem = accountProblem(values, t, { password: false });
    if (problem) {
      setIssue(problem);
      return;
    }
    setPending(true);
    setIssue(null);
    const refused = await onSubmit({
      displayName: values.displayName.trim(),
      username: values.username.trim(),
      email: values.email.trim(),
    });
    setPending(false);
    setIssue(refused);
  }

  return (
    <AuthFrame
      scene="register"
      title={t("auth.providerSignUp.title", { provider })}
      description={
        invite === null
          ? t("auth.register.description")
          : t("auth.register.descriptionInvite", { server: invite.server, inviter: invite.inviter })
      }
    >
      <form ref={form} className={styles.form} onSubmit={(event) => void submit(event)} noValidate>
        <AccountFields
          t={t}
          values={values}
          onChange={change}
          issue={issue}
          errorId={errorId}
          disabled={pending}
          previewTag={t("auth.register.previewTag")}
          password={false}
        />
        <p className={styles.hint}>{t("auth.providerSignUp.noPassword", { provider })}</p>
        {issue ? <FormError id={errorId}>{issue.message}</FormError> : null}
        <button className={`${styles.button} ${styles.primary} ${styles.wide}`} type="submit" disabled={pending}>
          {pending ? t("auth.register.pending") : t("auth.register.submit")}
        </button>
      </form>
      <div className={styles.foot}>
        <p>{t("auth.providerSignUp.haveAccount", { provider })}</p>
        <Link className={styles.link} href="/login?oidc=unlinked">
          {t("auth.register.logIn")}
        </Link>
      </div>
    </AuthFrame>
  );
}

/**
 * /register without an invite. Accounts here come from invites, so instead
 * of a form the server would refuse, the page asks for the link.
 */
export function InviteNeeded({
  t,
  instanceName,
  onOpen,
  unusable = false,
}: {
  t: Translator;
  instanceName: string | null;
  onOpen(code: string): void;
  /** The invite this tab remembered no longer works. */
  unusable?: boolean;
}) {
  return (
    <AuthFrame
      scene="register"
      title={t("auth.join.title")}
      description={
        instanceName === null ? t("auth.join.description") : t("auth.join.descriptionNamed", { instance: instanceName })
      }
    >
      {unusable ? <FormError>{t("auth.register.inviteRefused")}</FormError> : null}
      <InviteLinkForm t={t} onOpen={onOpen} />
      <div className={styles.foot}>
        <p>{t("auth.join.haveAccount")}</p>
        <Link className={styles.link} href="/login">
          {t("auth.register.logIn")}
        </Link>
      </div>
    </AuthFrame>
  );
}
