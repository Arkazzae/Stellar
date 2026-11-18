import { Icon } from "@stellarr/icons";
import { useRouter } from "@stellarr/router";
import { useEffect, useId, useRef, useState, type FormEvent } from "react";

import {
  AccountFields,
  accountProblem,
  focusField,
  useAccountValues,
  type FormIssue,
} from "@/components/auth/AccountFields";
import controls from "@/components/auth/AuthControls.module.css";
import { AuthField, FormError } from "@/components/auth/AuthField";
import { AuthFrame } from "@/components/auth/AuthFrame";
import { Chip, ServerTile } from "@/components/auth/Faces";
import { ServerUnreachable } from "@/components/auth/Outcomes";
import { StrengthBars } from "@/components/auth/StrengthMeter";
import { Switch } from "@/components/primitives/Switch";
import { registrationIssue, type RegistrationField } from "@/containers/registrationErrors";
import type { Translator } from "@/i18n";
import { serverMark } from "@/state/directoryView";
import { useInstanceSetup, useLanguage, usePorts, useSession } from "@/state/hooks";

import { usePrefetchApp } from "./loadApp";

interface ServerDraft {
  name: string;
  description: string;
  textChannelName: string;
  voice: boolean;
  voiceChannelName: string;
}

type ServerField = "name" | "description" | "textChannelName" | "voiceChannelName";

/** The fields of the first step, where a refusal of any of them sends the person back. */
const ACCOUNT_STEP: readonly RegistrationField[] = ["setupCode", "displayName", "username", "email", "password"];

/** A channel name as the server stores it: trimmed and in lower case. */
const channelName = (typed: string) => typed.trim().toLowerCase();
const validChannel = (typed: string) => /^[\p{L}\p{N}-]{1,60}$/u.test(channelName(typed));

function serverProblem(server: ServerDraft, t: Translator): FormIssue<ServerField> | null {
  const name = server.name.trim();
  if (name.length === 0 || name.length > 80) return { field: "name", message: t("auth.setup.invalidName") };
  if (!validChannel(server.textChannelName)) {
    return { field: "textChannelName", message: t("auth.setup.invalidChannel") };
  }
  if (server.voice && !validChannel(server.voiceChannelName)) {
    return { field: "voiceChannelName", message: t("auth.setup.invalidChannel") };
  }
  if (server.voice && channelName(server.textChannelName) === channelName(server.voiceChannelName)) {
    return { field: "voiceChannelName", message: t("auth.setup.collision") };
  }
  return null;
}

function Steps({ t, step }: { t: Translator; step: 1 | 2 | 3 }) {
  const names = [t("auth.setup.stepOwner"), t("auth.setup.stepServer"), t("auth.setup.stepInvite")];
  return (
    <p className={controls.steps}>
      <StrengthBars lit={step} />
      <span>
        <strong>{t("auth.setup.step", { step, steps: 3 })}</strong> · {names[step - 1]}
      </span>
    </p>
  );
}

type InviteLink = { state: "loading" } | { state: "ready"; url: string } | { state: "failed" };

/**
 * The first run of an instance: the owner's account, then the first server,
 * then a link to invite people. The account and the server are made by one
 * registration at the end of the second step, so a tab closed after the
 * first leaves nothing half made. The code the API writes to its log at
 * start proves the person at the keyboard runs the instance.
 *
 * Every other address leads here while the instance has no account; once it
 * has one, this page leads to the sign-in page, or home for a session.
 */
export default function SetupPage() {
  const ports = usePorts();
  const router = useRouter();
  const { phase, reload } = useSession();
  const setup = useInstanceSetup();
  const { t } = useLanguage();
  const errorId = useId();
  const form = useRef<HTMLFormElement>(null);
  const account = useAccountValues();
  const [step, setStep] = useState<1 | 2 | 3>(1);
  const [setupCode, setSetupCode] = useState("");
  const [server, setServer] = useState<ServerDraft>(() => ({
    name: "",
    description: "",
    textChannelName: t("auth.setup.defaultText"),
    voice: true,
    voiceChannelName: t("auth.setup.defaultVoice"),
  }));
  const [issue, setIssue] = useState<FormIssue<string> | null>(null);
  const [pending, setPending] = useState(false);
  const [founded, setFounded] = useState(false);
  const [invite, setInvite] = useState<InviteLink>({ state: "loading" });
  const [copied, setCopied] = useState(false);
  usePrefetchApp(founded);

  // Decided only until this page has founded the instance: from then on the
  // flag says it is set up and the session may say signed in, and the page
  // still has its last step to show.
  const leaveFor = founded
    ? null
    : phase === "ready"
      ? "/"
      : phase === "anonymous" && setup.setupRequired === false
        ? "/login"
        : null;
  useEffect(() => {
    if (leaveFor !== null) router.replace(leaveFor);
  }, [leaveFor, router]);

  // A refusal can send the person back a step; the field it names takes the focus there.
  useEffect(() => {
    if (issue?.field) focusField(form.current, issue.field);
  }, [issue, step]);

  if (!founded) {
    if (phase === "error") return <ServerUnreachable t={t} onRetry={reload} />;
    if (phase === "anonymous" && setup.failed) return <ServerUnreachable t={t} onRetry={setup.retry} />;
    if (phase !== "anonymous" || setup.setupRequired !== true) {
      return (
        <AuthFrame scene="setup-owner" title={t("auth.setup.checkingTitle")} description={t("auth.setup.checking")}>
          {null}
        </AuthFrame>
      );
    }
  }

  const status = (field: string) => ({
    invalid: issue?.field === field,
    errorId: issue?.field === field ? errorId : undefined,
  });
  const changeServer = (patch: Partial<ServerDraft>) => setServer((current) => ({ ...current, ...patch }));

  const toServer = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const problem = setupCode.trim()
      ? accountProblem(account.values, t)
      : { field: "setupCode", message: t("auth.setup.codeMissing") };
    setIssue(problem);
    if (problem === null) setStep(2);
    else focusField(form.current, problem.field ?? "");
  };

  const create = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (pending) return;
    const problem = serverProblem(server, t);
    setIssue(problem);
    if (problem !== null) {
      focusField(form.current, problem.field ?? "");
      return;
    }

    setPending(true);
    try {
      const result = await ports.session.register({
        displayName: account.values.displayName.trim(),
        username: account.values.username.trim(),
        email: account.values.email.trim(),
        password: account.values.password,
        setupCode: setupCode.trim(),
        initialServer: {
          name: server.name.trim(),
          description: server.description.trim(),
          textChannelName: channelName(server.textChannelName),
          voiceChannelName: server.voice ? channelName(server.voiceChannelName) : "",
        },
      });
      setFounded(true);
      setup.markSetUp();
      if (result.kind !== "signed-in") {
        // The instance asks for a second step: the account exists, the
        // session does not yet, and the sign-in page resumes the setup from
        // the challenge the server keeps for this browser.
        router.replace("/login");
        return;
      }
      setStep(3);
      ports.admin.createInvite().then(
        (created) => setInvite({ state: "ready", url: created.url }),
        () => setInvite({ state: "failed" }),
      );
    } catch (cause) {
      const refused = registrationIssue(cause, t);
      setIssue(refused);
      if (refused.field !== null && ACCOUNT_STEP.includes(refused.field)) setStep(1);
    } finally {
      setPending(false);
    }
  };

  const copy = async (url: string) => {
    try {
      await navigator.clipboard.writeText(url);
      setCopied(true);
    } catch {
      // The link stays on screen to select by hand.
    }
  };

  if (step === 3) {
    const name = server.name.trim();
    return (
      <AuthFrame
        scene="setup-invite"
        headingAction={<Steps t={t} step={3} />}
        title={t("auth.setup.inviteTitle")}
        description={t("auth.setup.inviteDescription", { server: name })}
      >
        {invite.state === "failed" ? (
          <FormError>{t("auth.setup.inviteFailed")}</FormError>
        ) : (
          <div className={controls.field}>
            <span className={controls.label} id={`${errorId}-link`}>
              {t("auth.invite.linkLabel")}
            </span>
            <div className={controls.linkBox}>
              <output className={controls.mono} aria-labelledby={`${errorId}-link`}>
                {invite.state === "ready" ? invite.url : t("auth.setup.inviteMaking")}
              </output>
              <button
                type="button"
                className={`${controls.button} ${copied ? controls.secondary : controls.primary}`}
                onClick={() => invite.state === "ready" && void copy(invite.url)}
                disabled={invite.state !== "ready"}
              >
                <Icon name={copied ? "check" : "copy"} size={18} />
                <span>{copied ? t("auth.setup.copied") : t("auth.setup.copy")}</span>
              </button>
            </div>
            <p className={controls.hint}>{t("auth.setup.inviteHint")}</p>
          </div>
        )}
        <button
          type="button"
          className={`${controls.button} ${invite.state === "failed" ? controls.primary : controls.secondary} ${controls.wide}`}
          onClick={() => {
            reload();
            router.replace("/");
          }}
        >
          {t("auth.setup.goToServer")}
        </button>
      </AuthFrame>
    );
  }

  if (step === 2) {
    const name = server.name.trim();
    return (
      <AuthFrame
        scene="setup-server"
        headingAction={<Steps t={t} step={2} />}
        title={t("auth.setup.serverTitle")}
        description={t("auth.setup.serverDescription")}
      >
        <Chip
          className={controls.preview}
          face={<ServerTile mark={name === "" ? null : serverMark(name)} />}
          name={name || t("auth.setup.emptyName")}
          placeholder={name === ""}
          detail={
            <span className={controls.channels}>
              <span>
                <Icon name="hash" size={14} />
                {channelName(server.textChannelName) || t("auth.setup.channelPlaceholder")}
              </span>
              {server.voice ? (
                <span>
                  <Icon name="speaker" size={14} />
                  {channelName(server.voiceChannelName) || t("auth.setup.channelPlaceholder")}
                </span>
              ) : null}
            </span>
          }
          end={<span className={controls.tag}>{t("auth.setup.preview")}</span>}
        />
        <form ref={form} className={controls.form} onSubmit={(event) => void create(event)} noValidate>
          <AuthField
            label={t("auth.setup.name")}
            name="name"
            placeholder={t("auth.setup.namePlaceholder")}
            maxLength={80}
            value={server.name}
            onChange={(event) => changeServer({ name: event.target.value })}
            disabled={pending}
            {...status("name")}
            {...(issue?.field === "server" ? { invalid: true, errorId } : {})}
          />
          <AuthField
            label={t("auth.setup.descriptionLabel")}
            labelNote={t("auth.setup.optional")}
            name="description"
            placeholder={t("auth.setup.descriptionPlaceholder")}
            maxLength={280}
            value={server.description}
            onChange={(event) => changeServer({ description: event.target.value })}
            disabled={pending}
          />
          <AuthField
            label={t("auth.setup.textChannel")}
            hint={t("auth.setup.channelHint")}
            name="textChannelName"
            lead={<Icon name="hash" size={18} />}
            maxLength={60}
            value={server.textChannelName}
            onChange={(event) => changeServer({ textChannelName: event.target.value })}
            disabled={pending}
            {...status("textChannelName")}
            {...(issue?.field === "channels" ? { invalid: true, errorId } : {})}
          />
          <div className={controls.switchRow}>
            <span aria-hidden="true">{t("auth.setup.voice")}</span>
            <Switch
              checked={server.voice}
              onChange={(voice) => changeServer({ voice })}
              label={t("auth.setup.voice")}
              disabled={pending}
            />
          </div>
          {server.voice ? (
            <AuthField
              label={t("auth.setup.voiceChannel")}
              name="voiceChannelName"
              lead={<Icon name="speaker" size={18} />}
              maxLength={60}
              value={server.voiceChannelName}
              onChange={(event) => changeServer({ voiceChannelName: event.target.value })}
              disabled={pending}
              {...status("voiceChannelName")}
            />
          ) : null}
          {issue ? <FormError id={errorId}>{issue.message}</FormError> : null}
          <button className={`${controls.button} ${controls.primary} ${controls.wide}`} type="submit" disabled={pending}>
            {pending ? t("auth.setup.pending") : t("auth.setup.create")}
          </button>
        </form>
        <div>
          <button
            type="button"
            className={controls.link}
            onClick={() => {
              setIssue(null);
              setStep(1);
            }}
            disabled={pending}
          >
            {t("auth.setup.back")}
          </button>
        </div>
      </AuthFrame>
    );
  }

  return (
    <AuthFrame
      scene="setup-owner"
      headingAction={<Steps t={t} step={1} />}
      title={t("auth.setup.ownerTitle")}
      description={t("auth.setup.ownerDescription", { host: window.location.host })}
    >
      <form ref={form} className={controls.form} onSubmit={toServer} noValidate>
        <AuthField
          label={t("auth.setup.code")}
          hint={t("auth.setup.codeHint")}
          name="setupCode"
          className={controls.mono}
          placeholder="XXXX-XXXX-XXXX"
          autoComplete="off"
          autoCapitalize="characters"
          spellCheck={false}
          maxLength={32}
          value={setupCode}
          onChange={(event) => setSetupCode(event.target.value)}
          {...status("setupCode")}
        />
        {/* The code is first and the form is long: its refusal is said beside it. */}
        {issue?.field === "setupCode" ? <FormError id={errorId}>{issue.message}</FormError> : null}
        <AccountFields
          t={t}
          values={account.values}
          onChange={account.change}
          issue={issue}
          errorId={errorId}
          disabled={false}
          previewTag={t("auth.setup.ownerTag")}
        />
        {issue && issue.field !== "setupCode" ? <FormError id={errorId}>{issue.message}</FormError> : null}
        <button className={`${controls.button} ${controls.primary} ${controls.wide}`} type="submit">
          {t("auth.setup.continue")}
        </button>
      </form>
    </AuthFrame>
  );
}
