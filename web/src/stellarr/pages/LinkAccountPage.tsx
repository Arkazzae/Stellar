import { Link, useSearchParams } from "@stellarr/router";

import controls from "@/components/auth/AuthControls.module.css";
import { AuthFrame } from "@/components/auth/AuthFrame";
import type { MessageKey } from "@/i18n";
import { singleQueryValue } from "@/lib/auth/query";
import { useLanguage } from "@/state/hooks";

type LinkOutcome = "linked" | "taken" | "other" | "cancelled" | "failed";

const OUTCOMES: readonly LinkOutcome[] = ["linked", "taken", "other", "cancelled", "failed"];

const DESCRIPTION: Record<LinkOutcome, MessageKey> = {
  linked: "linkAccount.linked",
  taken: "linkAccount.taken",
  other: "linkAccount.other",
  cancelled: "linkAccount.cancelled",
  failed: "linkAccount.failed",
};

/**
 * Where the identity provider sends the browser back after a link started in
 * the settings. The result is read from the address in the browser; anything
 * the server did not send, a repeated result included, reads as a failure,
 * never as a success.
 */
export default function LinkAccountPage() {
  const result = singleQueryValue(useSearchParams(), "result");
  const outcome = OUTCOMES.find((candidate) => candidate === result) ?? "failed";
  const { t } = useLanguage();
  return (
    <AuthFrame
      outcome={outcome === "linked" ? "success" : "failure"}
      title={outcome === "linked" ? t("linkAccount.linkedTitle") : t("linkAccount.failedTitle")}
      description={t(DESCRIPTION[outcome])}
    >
      <Link className={`${controls.button} ${controls.primary} ${controls.wide}`} href="/">
        {t("linkAccount.back")}
      </Link>
    </AuthFrame>
  );
}
