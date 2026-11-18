import { Icon } from "@stellarr/icons";

import { FormError } from "@/components/auth/AuthField";
import { AuthFrame, type StepFrame } from "@/components/auth/AuthFrame";
import type { Translator } from "@/i18n";
import type { IdentityProvider } from "@/ports";

import styles from "./AuthControls.module.css";

export interface ProviderConfirmStepProps {
  t: Translator;
  /**
   * The configured providers the account is linked to; any of them can
   * confirm. None when every one it was linked to is gone from the instance.
   */
  providers: IdentityProvider[];
  pending: boolean;
  error: string | null;
  /** Leaves for the provider, which sends the browser back afterwards. */
  onConfirm(provider: IdentityProvider): void;
  /** Defaults to the sign-in page. */
  frame?: StepFrame;
  /** A way out, offered on the page; a dialog has its own close button. */
  onBack?(): void;
}

/**
 * The proof an account without a password gives when its sign-in is more
 * than a few minutes old and it has no second step: a fresh sign-in at one
 * of its providers. The browser leaves for the provider and comes back to a
 * page that says whether it worked, so the change is made again from there.
 */
export function ProviderConfirmStep({
  t,
  providers,
  pending,
  error,
  onConfirm,
  frame: Frame = AuthFrame,
  onBack,
}: ProviderConfirmStepProps) {
  return (
    <Frame title={t("security.provider.title")} description={t("security.provider.description")} scene="second-step">
      <div className={styles.stack}>
        {providers.length === 0 ? <p className={styles.hint}>{t("security.provider.none")}</p> : null}
        {providers.map((provider) => (
          <button
            key={provider.id}
            type="button"
            className={`${styles.button} ${styles.primary} ${styles.wide}`}
            onClick={() => onConfirm(provider)}
            disabled={pending}
          >
            <Icon name="login" size={20} />
            <span>{t("security.provider.submit", { provider: provider.displayName })}</span>
          </button>
        ))}
        {error ? <FormError>{error}</FormError> : null}
      </div>
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
