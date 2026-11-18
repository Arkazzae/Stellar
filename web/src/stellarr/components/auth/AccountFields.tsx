import { useCallback, useState } from "react";

import { avatarColor } from "@/design/identity";
import type { Translator } from "@/i18n";
import { MIN_PASSWORD_LENGTH } from "@/lib/auth/passwordStrength";
import { isValidUsername, suggestUsername, USERNAME_MAX_LENGTH } from "@/lib/auth/username";

import { AuthField, PasswordField } from "./AuthField";
import styles from "./AuthControls.module.css";
import { Chip, Face } from "./Faces";
import { StrengthMeter } from "./StrengthMeter";

/** Each value is also the `name` of its input, which is how focus finds it. */
export type AccountField = "displayName" | "username" | "email" | "password";

export type AccountValues = Record<AccountField, string>;

const EMPTY: AccountValues = { displayName: "", username: "", email: "", password: "" };

const MAX_PASSWORD_LENGTH = 256;

/** What the form or the server found wrong, and with which field, if any. */
export interface FormIssue<F extends string = string> {
  field: F | null;
  message: string;
}

/**
 * The values of a new account. The login follows the display name, made
 * into a plain login, until the person types one of their own; emptying the
 * field hands it back to the suggestion.
 */
export function useAccountValues(): {
  values: AccountValues;
  change(field: AccountField, value: string): void;
} {
  const [values, setValues] = useState<AccountValues>(EMPTY);
  const [ownUsername, setOwnUsername] = useState(false);

  const change = useCallback(
    (field: AccountField, value: string) => {
      if (field === "username") setOwnUsername(value.length > 0);
      setValues((current) =>
        field === "displayName" && !ownUsername
          ? { ...current, displayName: value, username: suggestUsername(value) }
          : { ...current, [field]: value },
      );
    },
    [ownUsername],
  );

  return { values, change };
}

/** The first problem in visual order, so repeated submits walk down the form. */
export function accountProblem(values: AccountValues, t: Translator): FormIssue<AccountField> | null {
  const displayName = values.displayName.trim();
  if (displayName.length === 0 || displayName.length > 64) {
    return { field: "displayName", message: t("auth.register.displayNameInvalid") };
  }
  if (!isValidUsername(values.username.trim())) {
    return { field: "username", message: t("auth.register.usernameInvalid") };
  }
  if (!/^[^\s@]+@[^\s@]+$/.test(values.email.trim())) {
    return { field: "email", message: t("auth.register.emailInvalid") };
  }
  if (values.password.length < MIN_PASSWORD_LENGTH || values.password.length > MAX_PASSWORD_LENGTH) {
    return { field: "password", message: t("auth.password.length") };
  }
  return null;
}

/**
 * The fields of a new account, with a line under the name that shows how
 * others will see it: the face, the name and the login, as typed. It tells
 * the display name from the login better than a hint could.
 *
 * No field asks whether a login is free while it is typed: that would let
 * anyone find out who has an account. A taken login is said after sending.
 */
export function AccountFields({
  t,
  values,
  onChange,
  issue,
  errorId,
  disabled,
  previewTag,
}: {
  t: Translator;
  values: AccountValues;
  onChange(field: AccountField, value: string): void;
  issue: FormIssue | null;
  errorId: string;
  disabled: boolean;
  /** What the preview line is: how others will see you, or the owner. */
  previewTag: string;
}) {
  const status = (field: AccountField) => ({
    invalid: issue?.field === field,
    errorId: issue?.field === field ? errorId : undefined,
  });
  const name = values.displayName.trim();
  const login = values.username.trim();

  return (
    <>
      <AuthField
        label={t("auth.register.displayName")}
        name="displayName"
        autoComplete="nickname"
        placeholder={t("auth.register.displayNamePlaceholder")}
        maxLength={64}
        value={values.displayName}
        onChange={(event) => onChange("displayName", event.target.value)}
        disabled={disabled}
        {...status("displayName")}
      />
      <Chip
        className={styles.preview}
        face={<Face name={name || login} color={avatarColor(login || name)} />}
        name={name || t("auth.register.previewName")}
        placeholder={name === ""}
        detail={<span className={styles.mono}>@{login || t("auth.register.previewLogin")}</span>}
        end={<span className={styles.tag}>{previewTag}</span>}
      />
      <AuthField
        label={t("auth.register.username")}
        hint={t("auth.register.usernameHint")}
        name="username"
        lead={<span className={styles.mono}>@</span>}
        className={styles.mono}
        autoComplete="username"
        autoCapitalize="none"
        spellCheck={false}
        maxLength={USERNAME_MAX_LENGTH}
        value={values.username}
        onChange={(event) => onChange("username", event.target.value)}
        disabled={disabled}
        {...status("username")}
      />
      <AuthField
        label={t("auth.email")}
        hint={t("auth.register.emailHint")}
        name="email"
        type="email"
        autoComplete="email"
        value={values.email}
        onChange={(event) => onChange("email", event.target.value)}
        disabled={disabled}
        {...status("email")}
      />
      <PasswordField
        t={t}
        label={t("auth.password")}
        name="password"
        autoComplete="new-password"
        value={values.password}
        onChange={(event) => onChange("password", event.target.value)}
        disabled={disabled}
        {...status("password")}
      />
      <StrengthMeter password={values.password} username={values.username} t={t} />
    </>
  );
}

/** Puts the focus on a form's field by name; on a phone it is also what scrolls to it. */
export function focusField(form: HTMLFormElement | null, field: string): void {
  const input = form?.elements.namedItem(field);
  if (input instanceof HTMLInputElement) input.focus();
}
