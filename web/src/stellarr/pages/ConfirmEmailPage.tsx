"use client";

import { Link, useSearchParams } from "@stellarr/router";
import { useState } from "react";

import { AuthFrame, authStyles as styles } from "@/components/auth/AuthFrame";
import type { MessageKey } from "@/i18n";
import { singleQueryValue } from "@/lib/auth/query";
import { useLanguage, usePorts } from "@/state/hooks";

type Outcome = "changed" | "invalid" | "taken" | "failed";

const TITLE: Record<Outcome, MessageKey> = {
  changed: "confirmEmail.changedTitle",
  invalid: "confirmEmail.notChangedTitle",
  taken: "confirmEmail.notChangedTitle",
  failed: "confirmEmail.notChangedTitle",
};

const DESCRIPTION: Record<Outcome, MessageKey> = {
  changed: "confirmEmail.changed",
  invalid: "confirmEmail.invalid",
  taken: "confirmEmail.taken",
  failed: "confirmEmail.failed",
};

/**
 * Where the link from the new address leads. It changes nothing on arrival:
 * mail scanners open links on their own, so the address changes only when
 * the person presses the button.
 *
 * The token is read from the address in the browser. A missing or repeated
 * one reads as an invalid link, since no mailed link produces it.
 */
export default function ConfirmEmailPage() {
  const token = singleQueryValue(useSearchParams(), "token") ?? "";
  const ports = usePorts();
  const { t } = useLanguage();
  const [pending, setPending] = useState(false);
  const [outcome, setOutcome] = useState<Outcome | null>(token.length === 0 ? "invalid" : null);

  const confirm = async () => {
    setPending(true);
    try {
      setOutcome(await ports.session.confirmEmailChange(token));
    } catch {
      setOutcome("failed");
    } finally {
      setPending(false);
    }
  };

  if (outcome === null) {
    return (
      <AuthFrame title={t("confirmEmail.title")} description={t("confirmEmail.description")}>
        <button type="button" className={styles.submit} onClick={() => void confirm()} disabled={pending}>
          {pending ? t("common.saving") : t("confirmEmail.confirm")}
        </button>
      </AuthFrame>
    );
  }

  return (
    <AuthFrame title={t(TITLE[outcome])} description={t(DESCRIPTION[outcome])}>
      {outcome === "failed" ? (
        <button type="button" className={styles.submit} onClick={() => void confirm()} disabled={pending}>
          {t("confirmEmail.retry")}
        </button>
      ) : (
        <Link className={styles.submit} href="/">
          {t("linkAccount.back")}
        </Link>
      )}
    </AuthFrame>
  );
}
