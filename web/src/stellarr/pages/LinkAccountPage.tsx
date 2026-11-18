import { Link, useSearchParams } from "@stellarr/router";

import controls from "@/components/auth/AuthControls.module.css";
import { AuthFrame } from "@/components/auth/AuthFrame";
import type { MessageKey } from "@/i18n";
import { singleQueryValue } from "@/lib/auth/query";
import { safeReturnPath } from "@/lib/auth/returnPath";
import { useLanguage } from "@/state/hooks";

/**
 * How a round trip to an identity provider came out: a link, from the
 * settings or from a password sign-in that asked for one, or a fresh sign-in
 * at the provider asked for as proof (confirmed or unconfirmed).
 */
type LinkOutcome = "linked" | "taken" | "other" | "cancelled" | "failed" | "confirmed" | "unconfirmed";

const OUTCOMES: readonly LinkOutcome[] = ["linked", "taken", "other", "cancelled", "failed", "confirmed", "unconfirmed"];

const TITLE: Record<LinkOutcome, MessageKey> = {
  linked: "linkAccount.linkedTitle",
  taken: "linkAccount.failedTitle",
  other: "linkAccount.failedTitle",
  cancelled: "linkAccount.failedTitle",
  failed: "linkAccount.failedTitle",
  confirmed: "linkAccount.confirmedTitle",
  unconfirmed: "linkAccount.unconfirmedTitle",
};

const DESCRIPTION: Record<LinkOutcome, MessageKey> = {
  linked: "linkAccount.linked",
  taken: "linkAccount.taken",
  other: "linkAccount.other",
  cancelled: "linkAccount.cancelled",
  failed: "linkAccount.failed",
  confirmed: "linkAccount.confirmed",
  unconfirmed: "linkAccount.unconfirmed",
};

/**
 * Where the identity provider sends the browser back after a link or a
 * fresh sign-in started in the settings, and where a password sign-in that
 * linked an identity says how that went, once. The result is read from the
 * address in the browser; anything the server did not send, a repeated
 * result included, reads as a failure, never as a success. The way on is
 * the page the round trip was started from, when the address names one on
 * this origin, else the app.
 */
export default function LinkAccountPage() {
  const params = useSearchParams();
  const result = singleQueryValue(params, "result");
  const outcome = OUTCOMES.find((candidate) => candidate === result) ?? "failed";
  const back = safeReturnPath(singleQueryValue(params, "returnUrl"));
  const { t } = useLanguage();
  const action = `${controls.button} ${controls.primary} ${controls.wide}`;
  return (
    <AuthFrame
      outcome={outcome === "linked" || outcome === "confirmed" ? "success" : "failure"}
      title={t(TITLE[outcome])}
      description={t(DESCRIPTION[outcome])}
    >
      {back === null ? (
        <Link className={action} href="/">
          {t("linkAccount.back")}
        </Link>
      ) : (
        // A full load: the page may be outside this app, such as Cockpit.
        <a className={action} href={back}>
          {t("linkAccount.back")}
        </a>
      )}
    </AuthFrame>
  );
}
