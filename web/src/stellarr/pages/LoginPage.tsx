import { useRouter, useSearchParams } from "@stellarr/router";
import { useEffect, useState } from "react";

import { AuthFrame } from "@/components/auth/AuthFrame";
import controls from "@/components/auth/AuthControls.module.css";
import { LoginScreen } from "@/components/auth/LoginScreen";
import { MfaChallengeStep } from "@/components/auth/MfaChallengeStep";
import { MfaEnrollStep } from "@/components/auth/MfaEnrollStep";
import { ServerUnreachable } from "@/components/auth/Outcomes";
import { PasswordConfirmStep } from "@/components/auth/PasswordConfirmStep";
import { QrCode } from "@/components/auth/QrCode";
import { RecoveryCodesStep } from "@/components/auth/RecoveryCodesStep";
import { RecoveryRedeemStep } from "@/components/auth/RecoveryRedeemStep";
import { useSignInFlow } from "@/containers/useSignInFlow";
import { useSignInProviders, useInstanceName } from "@/hooks/useInstanceFacts";
import { useRedirectWhenSignedIn } from "@/hooks/useRedirectWhenSignedIn";
import type { Translator } from "@/i18n";
import { safeReturnPath } from "@/lib/auth/returnPath";
import { useInstanceSetup, useLanguage, usePorts, useSession } from "@/state/hooks";

import { usePrefetchApp } from "./loadApp";

/**
 * The identity provider confirmed someone the instance cannot sign in, or
 * the round trip failed. Either way the password form is the way on, and
 * the page says so before it shows it.
 */
function ProviderProblem({
  t,
  notLinked,
  instanceName,
  onContinue,
}: {
  t: Translator;
  notLinked: boolean;
  instanceName: string | null;
  onContinue(): void;
}) {
  return (
    <AuthFrame
      outcome="failure"
      title={notLinked ? t("auth.oidc.notLinkedTitle") : t("auth.oidc.failedTitle")}
      description={
        notLinked
          ? instanceName === null
            ? t("auth.oidc.notLinked")
            : t("auth.oidc.notLinkedNamed", { instance: instanceName })
          : t("auth.oidc.failed")
      }
    >
      <button type="button" className={`${controls.button} ${controls.primary} ${controls.wide}`} onClick={onContinue}>
        {t("auth.oidc.usePassword")}
      </button>
    </AuthFrame>
  );
}

export default function LoginPage() {
  const { phase, loginPending, loginError, reload } = useSession();
  const setup = useInstanceSetup();
  const { t } = useLanguage();
  const router = useRouter();
  const params = useSearchParams();
  const oidcError = params.get("oidc_error");
  const returnPath = safeReturnPath(params.get("returnUrl"));
  usePrefetchApp(phase !== "loading");
  const providers = useSignInProviders();
  const instanceName = useInstanceName();
  const ports = usePorts();
  const [passkeyOffered, setPasskeyOffered] = useState(false);
  const confirm = params.get("confirm") === "1";
  // A recovery after the identity provider says which choice it left with.
  const [keepSignedIn, setKeepSignedIn] = useState(params.get("keep_signed_in") !== "0");
  const [providerProblemSeen, setProviderProblemSeen] = useState(false);
  const flow = useSignInFlow({
    recoveryRequired: params.get("security_recovery_required") === "1",
    confirm,
    fresh: params.get("fresh") === "1",
    keepSignedIn,
  });

  // A confirmation leaves only once the second step is done, not as soon as
  // the session is found to be signed in.
  useRedirectWhenSignedIn(confirm && flow.step.kind !== "done" ? "loading" : phase, returnPath);

  useEffect(() => {
    if (!flow.passkeysSupported) return;
    let active = true;
    // Offered only when the server can check passkeys; the password form stays either way.
    ports.session.passkeySignInAvailable().then(
      (available) => active && setPasskeyOffered(available),
      () => undefined,
    );
    return () => {
      active = false;
    };
  }, [flow.passkeysSupported, ports]);

  if (phase === "error") return <ServerUnreachable t={t} onRetry={reload} />;
  if (phase === "anonymous" && setup.failed) return <ServerUnreachable t={t} onRetry={setup.retry} />;

  if ((oidcError === "not_linked" || oidcError === "failed") && !providerProblemSeen) {
    return (
      <ProviderProblem
        t={t}
        notLinked={oidcError === "not_linked"}
        instanceName={instanceName}
        onContinue={() => setProviderProblemSeen(true)}
      />
    );
  }

  const { step } = flow;
  if (step.kind === "challenge") {
    return (
      <MfaChallengeStep
        challenge={step.challenge}
        t={t}
        pending={flow.pending}
        error={flow.error}
        onVerify={flow.verify}
        onUseTrustedDevice={flow.continueOnTrustedDevice}
        onSendEmailCode={flow.sendEmailCode}
        onCancel={flow.cancel}
        // Signed in already, the step only confirms who is there: going back
        // leaves for the app, so it is no way to switch accounts.
        showAccount={!confirm}
        // A trusted device cannot stand in for a fresh second step.
        offerTrust={step.challenge.purpose !== "reverify"}
        passkeysSupported={flow.passkeysSupported}
        onUsePasskey={flow.answerWithPasskey}
      />
    );
  }
  if (step.kind === "confirm-password") {
    return (
      <PasswordConfirmStep
        t={t}
        pending={flow.pending}
        error={flow.error}
        onSubmit={flow.confirmPassword}
        onBack={flow.cancel}
      />
    );
  }
  if (step.kind === "enroll") {
    return (
      <MfaEnrollStep
        challenge={step.challenge}
        t={t}
        mode={step.mode}
        enrollment={step.enrollment}
        emailRetryAfter={flow.enrollEmailWait}
        pending={flow.pending}
        error={flow.error}
        qr={
          step.enrollment?.otpAuthUri === undefined ? undefined : (
            <QrCode value={step.enrollment.otpAuthUri} label={t("auth.enroll.qrLabel")} />
          )
        }
        onModeChange={flow.setEnrollMode}
        onSendEmailCode={flow.sendEnrollEmailCode}
        onConfirm={flow.confirmEnrollment}
        onCancel={flow.cancel}
        passkeysSupported={flow.passkeysSupported}
        onCreatePasskey={flow.createPasskey}
      />
    );
  }
  if (step.kind === "codes") {
    return (
      <RecoveryCodesStep codes={step.codes} username={step.username} t={t} onDone={flow.finishCodes} />
    );
  }
  if (step.kind === "redeem") {
    return (
      <RecoveryRedeemStep
        t={t}
        pending={flow.pending}
        error={flow.error}
        onSubmit={flow.redeem}
        onBack={flow.cancel}
      />
    );
  }

  return (
    <LoginScreen
      t={t}
      instanceName={instanceName}
      // A login that needs nothing more re-runs the session bootstrap, and the
      // redirect hook navigates once it reports ready. Any further step stays
      // on this page.
      onSubmit={(username, password) => void flow.submitPassword(username, password, keepSignedIn)}
      keepSignedIn={{ checked: keepSignedIn, onChange: setKeepSignedIn }}
      passkeyLogin={
        passkeyOffered
          ? {
              label: t("auth.login.passkey"),
              onClick: () => flow.signInWithPasskey(keepSignedIn),
              pending: flow.pending,
            }
          : undefined
      }
      pending={loginPending}
      error={flow.error ?? loginError}
      checking={
        phase === "loading" ||
        // Until it is known whether the instance has an owner yet: a fresh
        // one leads to /setup, and this form would only flash past.
        (phase === "anonymous" && setup.setupRequired !== false) ||
        step.kind === "resuming" ||
        step.kind === "done" ||
        (confirm && phase === "ready" && flow.error === null)
      }
      externalLogins={providers.map((provider) => ({
        href: `${provider.loginUrl}?returnUrl=${encodeURIComponent(returnPath ?? "/")}${
          keepSignedIn ? "" : "&keepSignedIn=false"
        }`,
        name: provider.displayName,
        label: t("auth.login.oidc", { provider: provider.displayName }),
      }))}
      onInviteCode={(code) => router.push(`/invite/${encodeURIComponent(code)}`)}
    />
  );
}
