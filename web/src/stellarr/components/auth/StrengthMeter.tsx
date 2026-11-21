import { MarkBars } from "@/components/primitives/StepIndicator";
import type { MessageKey, Translator } from "@/i18n";
import { passwordStrength, type PasswordScore } from "@/lib/auth/passwordStrength";

import styles from "./AuthControls.module.css";

const LABEL: Record<Exclude<PasswordScore, 0>, MessageKey> = {
  1: "auth.strength.weak",
  2: "auth.strength.good",
  3: "auth.strength.strong",
};

const HINT = {
  short: "auth.strength.short",
  guessable: "auth.strength.guessable",
  length: "auth.strength.length",
} as const satisfies Record<string, MessageKey>;

/** The three bars of the mark, lit as far as the password deserves, and why not further. */
export function StrengthMeter({ password, username, t }: { password: string; username: string; t: Translator }) {
  const { score, hint } = passwordStrength(password, username);
  const label = score === 0 ? (hint === "short" ? t("auth.strength.tooShort") : null) : t(LABEL[score]);

  return (
    <p className={styles.meter} aria-live="polite">
      <MarkBars lit={score} />
      <span>
        {label === null ? (
          t("auth.strength.empty")
        ) : (
          <>
            <strong>{label}</strong>
            {hint === null ? null : ` · ${t(HINT[hint])}`}
          </>
        )}
      </span>
    </p>
  );
}
