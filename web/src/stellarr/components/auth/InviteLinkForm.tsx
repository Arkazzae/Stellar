import { useId, useState, type FormEvent } from "react";

import type { Translator } from "@/i18n";
import { inviteCodeFrom } from "@/lib/auth/invites";

import { FormError } from "./AuthField";
import styles from "./AuthControls.module.css";

/**
 * Where someone pastes the invite link they were sent, or just its code. It
 * opens the invite's own page, which says where the link leads; what is no
 * invite at all is refused here, with what to paste instead.
 */
export function InviteLinkForm({
  t,
  onOpen,
  autoFocus = false,
}: {
  t: Translator;
  onOpen(code: string): void;
  autoFocus?: boolean;
}) {
  const inputId = useId();
  const errorId = useId();
  const [text, setText] = useState("");
  const [refused, setRefused] = useState(false);
  const example = typeof window === "undefined" ? "" : `${window.location.host}/invite/…`;

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const code = inviteCodeFrom(text);
    setRefused(code === null);
    if (code !== null) onOpen(code);
  };

  return (
    <form className={styles.field} onSubmit={submit} noValidate>
      <label className={styles.label} htmlFor={inputId}>
        {t("auth.invite.linkLabel")}
      </label>
      <div className={styles.inlineRow}>
        <div className={styles.control} data-invalid={refused || undefined}>
          <input
            id={inputId}
            className={styles.mono}
            type="text"
            inputMode="url"
            autoComplete="off"
            autoCapitalize="none"
            spellCheck={false}
            placeholder={example}
            value={text}
            onChange={(event) => setText(event.target.value)}
            autoFocus={autoFocus}
            aria-invalid={refused || undefined}
            aria-describedby={refused ? errorId : undefined}
          />
        </div>
        <button className={`${styles.button} ${styles.secondary}`} type="submit">
          {t("auth.invite.open")}
        </button>
      </div>
      {refused ? <FormError id={errorId}>{t("auth.invite.notALink")}</FormError> : null}
    </form>
  );
}
