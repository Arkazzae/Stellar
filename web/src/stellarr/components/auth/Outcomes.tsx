import type { Translator } from "@/i18n";

import styles from "./AuthControls.module.css";
import { AuthFrame } from "./AuthFrame";

/**
 * The server did not answer, or answered with something the page cannot
 * use. A form now would only fail on submit, so the page says what happened
 * and offers the one thing that can help.
 */
export function ServerUnreachable({ t, onRetry }: { t: Translator; onRetry(): void }) {
  return (
    <AuthFrame outcome="offline" title={t("auth.offline.title")} description={t("auth.offline.description")}>
      <button type="button" className={`${styles.button} ${styles.primary} ${styles.wide}`} onClick={onRetry}>
        {t("auth.offline.retry")}
      </button>
    </AuthFrame>
  );
}
