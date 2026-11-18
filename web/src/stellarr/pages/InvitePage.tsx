import { Icon } from "@stellarr/icons";
import { Link, useParams, useRouter } from "@stellarr/router";
import { useEffect, useState } from "react";

import controls from "@/components/auth/AuthControls.module.css";
import { AuthFrame } from "@/components/auth/AuthFrame";
import { Face, ServerTile } from "@/components/auth/Faces";
import { ServerUnreachable } from "@/components/auth/Outcomes";
import { AppSkeleton } from "@/components/shell/AppSkeleton";
import { useSignInProviders } from "@/hooks/useInstanceFacts";
import { ApiError } from "@/lib/api/errors";
import { pendingInvite, rememberPendingInvite, takePendingInvite } from "@/lib/auth/invites";
import { formatDate } from "@/lib/format/format";
import type { InvitePreview } from "@/ports";
import { serverMark } from "@/state/directoryView";
import { useLanguage, usePorts, useSession } from "@/state/hooks";

import { usePrefetchApp } from "./loadApp";

type Preview =
  | { state: "loading" }
  | { state: "ready"; invite: InvitePreview }
  /** Unknown, expired, revoked, used up, or its server is gone: the server does not say which. */
  | { state: "unusable" }
  | { state: "failed" };

/** An invite that leads nowhere, with the one way on from here. */
function InviteUnusable({ signedIn }: { signedIn: boolean }) {
  const { t } = useLanguage();
  return (
    <AuthFrame outcome="failure" title={t("auth.invite.unusableTitle")} description={t("auth.invite.unusable")}>
      <Link className={`${controls.button} ${controls.primary} ${controls.wide}`} href={signedIn ? "/" : "/login"}>
        {signedIn ? t("auth.invite.backToApp") : t("auth.invite.backToLogin")}
      </Link>
    </AuthFrame>
  );
}

/**
 * What a signed-out visitor sees of an invite: whose server, who asks, which
 * channel and for how long, and the three ways in. Each remembers the invite
 * for this tab, so it is joined as soon as there is a session.
 */
function InviteDetails({ code, invite }: { code: string; invite: InvitePreview }) {
  const { t, locale } = useLanguage();
  const router = useRouter();
  const providers = useSignInProviders();
  const back = `/invite/${encodeURIComponent(code)}`;
  const go = (path: string) => {
    rememberPendingInvite(code);
    router.push(path);
  };

  return (
    <AuthFrame
      scene="invite"
      title={invite.server.name}
      description={invite.server.description}
      headingAction={
        <div className={controls.invite}>
          <ServerTile mark={serverMark(invite.server.name)} iconUrl={invite.server.iconUrl} large />
          <p className={controls.from}>
            <Face
              name={invite.inviter.displayName}
              color={invite.inviter.avatarColor}
              imageUrl={invite.inviter.avatarUrl}
              small
            />
            <span>{t("auth.invite.from", { inviter: invite.inviter.displayName })}</span>
          </p>
        </div>
      }
    >
      {invite.channel !== null || invite.expiresAt !== null ? (
        <ul className={controls.facts}>
          {invite.channel === null ? null : (
            <li>
              <Icon name="hash" size={15} />
              <span>{invite.channel.name}</span>
            </li>
          )}
          {invite.expiresAt === null ? null : (
            <li>
              <Icon name="clock" size={15} />
              <span>{t("auth.invite.validUntil", { date: formatDate(invite.expiresAt, locale) })}</span>
            </li>
          )}
        </ul>
      ) : null}
      <div className={controls.stack}>
        <button
          type="button"
          className={`${controls.button} ${controls.primary} ${controls.wide}`}
          onClick={() => go("/register")}
        >
          {t("auth.invite.register")}
        </button>
        <button
          type="button"
          className={`${controls.button} ${controls.secondary} ${controls.wide}`}
          onClick={() => go("/login")}
        >
          {t("auth.invite.login")}
        </button>
        {providers.map((provider) => (
          // The provider sends the visitor back here, signed in, and the page
          // joins. A second step on the way ends at /login, which takes the
          // remembered invite instead.
          <a
            key={provider.loginUrl}
            className={`${controls.button} ${controls.secondary} ${controls.wide}`}
            href={`${provider.loginUrl}?returnUrl=${encodeURIComponent(back)}`}
            onClick={() => rememberPendingInvite(code)}
          >
            <Icon name="login" size={20} />
            <span>{t("auth.invite.provider", { provider: provider.displayName })}</span>
          </a>
        ))}
      </div>
      <p className={controls.hint}>{t("auth.invite.after", { server: invite.server.name })}</p>
    </AuthFrame>
  );
}

/**
 * Where an invite link leads. Signed in, it joins the server and opens the
 * app, as before; signed out, it shows where the link leads and lets the
 * visitor create an account, log in or use an identity provider.
 *
 * The code is the last segment of the address, decoded, as the way back
 * from sign-in encodes it; an address whose code cannot be decoded fails like
 * a code the server refuses, without asking the server.
 */
export default function InvitePage() {
  const { code } = useParams<"code">();
  const router = useRouter();
  const ports = usePorts();
  const { phase, reload } = useSession();
  const [joinFailed, setJoinFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const asked = `${code ?? ""}#${attempt}`;
  const [answer, setAnswer] = useState<{ asked: string; preview: Preview } | null>(null);
  const preview: Preview = answer?.asked === asked ? answer.preview : { state: "loading" };
  const { t } = useLanguage();
  // Signed in, an invite ends in the app.
  usePrefetchApp(phase !== "loading");

  useEffect(() => {
    if (code === null || phase !== "ready") return;
    let active = true;
    // A visitor back from an identity provider lands here with the invite
    // still remembered; once it is answered, the next login must not bring
    // the tab back to it.
    const forget = () => {
      if (pendingInvite() === code) takePendingInvite();
    };
    ports.directory.joinInvite(code).then(
      (server) => {
        if (!active) return;
        forget();
        ports.directory.selectServer(server.id);
        router.replace("/");
        reload();
      },
      () => {
        if (!active) return;
        forget();
        setJoinFailed(true);
      },
    );
    return () => {
      active = false;
    };
  }, [code, phase, ports, reload, router]);

  useEffect(() => {
    if (code === null || phase !== "anonymous") return;
    let active = true;
    ports.session.invitePreview(code).then(
      (invite) => active && setAnswer({ asked, preview: { state: "ready", invite } }),
      (cause: unknown) => {
        if (!active) return;
        setAnswer({
          asked,
          preview: cause instanceof ApiError && cause.status === 404 ? { state: "unusable" } : { state: "failed" },
        });
      },
    );
    return () => {
      active = false;
    };
  }, [asked, code, phase, ports]);

  if (phase === "error") return <ServerUnreachable t={t} onRetry={reload} />;
  if (code === null) return <InviteUnusable signedIn={phase === "ready"} />;
  if (phase === "ready") return joinFailed ? <InviteUnusable signedIn /> : <AppSkeleton />;
  if (phase !== "anonymous") return <AppSkeleton />;

  switch (preview.state) {
    case "ready":
      return <InviteDetails code={code} invite={preview.invite} />;
    case "unusable":
      return <InviteUnusable signedIn={false} />;
    case "failed":
      return <ServerUnreachable t={t} onRetry={() => setAttempt((count) => count + 1)} />;
    case "loading":
      return (
        <AuthFrame scene="invite" title={t("auth.invite.checkingTitle")} description={t("auth.invite.checking")}>
          {null}
        </AuthFrame>
      );
  }
}
