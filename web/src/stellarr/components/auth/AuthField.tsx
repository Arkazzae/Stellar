import { Icon } from "@stellarr/icons";
import { useId, useState, type ComponentProps, type ReactNode } from "react";

import type { Translator } from "@/i18n";

import styles from "./AuthControls.module.css";

export interface AuthFieldProps extends ComponentProps<"input"> {
  label: string;
  /** A word after the label, such as "(optional)". */
  labelNote?: string;
  /** A link or button at the end of the label's line, such as "Forgot password". */
  labelAction?: ReactNode;
  hint?: string;
  invalid?: boolean;
  /** Pass this only while an error message naming the field is on screen. */
  errorId?: string;
  /** Drawn inside the box before the text, such as "@" or an icon. */
  lead?: ReactNode;
  /** A control inside the box after the text, such as "Show". */
  trailing?: ReactNode;
}

/** A labelled input of the account screens, with its hint and error wired up for assistive tech. */
export function AuthField({
  label,
  labelNote,
  labelAction,
  hint,
  invalid,
  errorId,
  lead,
  trailing,
  ...input
}: AuthFieldProps) {
  const inputId = useId();
  const hintId = useId();
  const describedBy = [hint ? hintId : undefined, errorId].filter(Boolean).join(" ");

  return (
    <div className={styles.field}>
      <div className={styles.labelRow}>
        <label className={styles.label} htmlFor={inputId}>
          {label}
          {labelNote ? <span className={styles.optional}> {labelNote}</span> : null}
        </label>
        {labelAction}
      </div>
      <div
        className={styles.control}
        data-invalid={invalid || undefined}
        data-disabled={input.disabled || undefined}
      >
        {lead ? (
          <span className={styles.lead} aria-hidden="true">
            {lead}
          </span>
        ) : null}
        <input
          {...input}
          id={inputId}
          aria-invalid={invalid || undefined}
          aria-describedby={describedBy || undefined}
        />
        {trailing}
      </div>
      {hint ? (
        <p className={styles.hint} id={hintId}>
          {hint}
        </p>
      ) : null}
    </div>
  );
}

export interface PasswordFieldProps extends Omit<AuthFieldProps, "type" | "trailing"> {
  t: Translator;
  /**
   * The desktop app's credential broker owns the password: the field is a
   * text field masked by its style, which the browser does not offer to
   * save, and "Show" lifts the mask instead of changing the type.
   */
  nativeCredentialBroker?: boolean;
}

/** A password with "Show", which lets a person check what they typed instead of typing it twice. */
export function PasswordField({ t, nativeCredentialBroker = false, className, autoComplete, ...field }: PasswordFieldProps) {
  const [shown, setShown] = useState(false);
  const masked = nativeCredentialBroker && !shown;

  return (
    <AuthField
      {...field}
      type={nativeCredentialBroker || shown ? "text" : "password"}
      autoComplete={nativeCredentialBroker ? "off" : autoComplete}
      autoCapitalize="none"
      spellCheck={false}
      className={[className, masked ? styles.masked : null].filter(Boolean).join(" ") || undefined}
      data-1p-ignore={nativeCredentialBroker || undefined}
      data-lpignore={nativeCredentialBroker || undefined}
      trailing={
        <button
          type="button"
          className={styles.toggle}
          aria-label={shown ? t("auth.password.hide") : t("auth.password.show")}
          onClick={() => setShown((current) => !current)}
          disabled={field.disabled}
        >
          {shown ? t("auth.password.hideShort") : t("auth.password.showShort")}
        </button>
      }
    />
  );
}

/** What went wrong and how to fix it, announced when it appears. */
export function FormError({ id, children }: { id?: string; children: ReactNode }) {
  return (
    <p className={styles.error} id={id} role="alert">
      <Icon name="warning" size={18} />
      <span>{children}</span>
    </p>
  );
}
