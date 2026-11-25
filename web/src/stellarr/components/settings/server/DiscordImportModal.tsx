import { useEffect, useId, useRef, useState, type FormEvent } from "react";

import { Icon } from "@stellarr/icons";
import { Button } from "@stellarr/ui";
import { DialogClose } from "@/components/primitives/DialogClose";
import { Modal } from "@/components/primitives/Modal";
import type { MessageKey } from "@/i18n";
import {
  DiscordImportRefusal,
  type DiscordHistory,
  type DiscordImportPort,
  type DiscordImportStatus,
  type DiscordPreview,
  type DiscordSourceServer,
} from "@/ports";
import { useLanguage } from "@/state/hooks";

import createStyles from "./ServerCreateModal.module.css";
import styles from "./DiscordImportModal.module.css";

const ERROR_MESSAGES = new Map<string, MessageKey>([
  ["discord-invalid-token", "discordImport.invalidToken"],
  ["discord-bot-access", "discordImport.botAccess"],
  ["discord-message-content", "discordImport.messageContent"],
  ["discord-history-restricted", "discordImport.historyRestricted"],
  ["discord-selection-changed", "discordImport.selectionChanged"],
  ["discord-no-history-channels", "discordImport.noHistory"],
  ["discord-invalid-selection", "discordImport.invalidSelection"],
  ["discord-rate-limited", "discordImport.rateLimited"],
  ["discord-import-busy", "discordImport.busy"],
  ["discord-interrupted", "discordImport.interrupted"],
  ["discord-write-failed", "discordImport.writeFailed"],
  ["discord-media-storage-limit", "discordImport.mediaStorageLimit"],
  ["discord-media-user-quota", "discordImport.mediaUserQuota"],
]);

const JOB_KEY = "stellarr.discord-import";

function rememberJob(id: string | null): void {
  try {
    if (id === null) window.sessionStorage.removeItem(JOB_KEY);
    else window.sessionStorage.setItem(JOB_KEY, id);
  } catch {
    // The dialog can still monitor the job until the tab is reloaded.
  }
}

function rememberedDiscordImport(): string | null {
  try {
    return window.sessionStorage.getItem(JOB_KEY);
  } catch {
    return null;
  }
}

type Step = "connect" | "source" | "channels" | "review" | "progress";
type Mode = DiscordHistory["type"];

interface Props {
  api: DiscordImportPort;
  onClose: () => void;
  onBack: () => void;
  onOpenServer: (id: string) => void;
}

export function DiscordImportModal({ api, onClose, onBack, onOpenServer }: Props) {
  const { locale, t } = useLanguage();
  const titleId = useId();
  const attachmentsTitleId = useId();
  const heading = useRef<HTMLHeadingElement>(null);
  const tokenInput = useRef<HTMLInputElement>(null);
  const pending = useRef<AbortController | null>(null);
  const [id, setId] = useState(rememberedDiscordImport);
  const [job, setJob] = useState<DiscordImportStatus | null>(null);
  const [step, setStep] = useState<Step>("progress");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<MessageKey | null>(null);
  const [pollFailed, setPollFailed] = useState(false);
  const [pollVersion, setPollVersion] = useState(0);
  const [showAttachments, setShowAttachments] = useState(false);

  const [token, setToken] = useState("");
  const [sources, setSources] = useState<DiscordSourceServer[]>([]);
  const [installUrl, setInstallUrl] = useState<string | null>(null);
  const [sourceId, setSourceId] = useState("");
  const [preview, setPreview] = useState<DiscordPreview | null>(null);
  const [name, setName] = useState("");
  const [selected, setSelected] = useState<string[]>([]);
  const [search, setSearch] = useState("");
  const [mode, setMode] = useState<Mode | null>(null);
  const [date, setDate] = useState("");
  const [openedAt] = useState(Date.now);
  const [threads, setThreads] = useState(true);

  const importState = job?.state;
  const pendingAttachments = job?.pendingAttachments ?? 0;
  const canRestoreAttachments = importState === "completed" && pendingAttachments > 0;
  const repairingAttachments = job?.report != null && importState !== "completed";
  const number = new Intl.NumberFormat(locale);
  let dialogTitle: MessageKey = "discordImport.title";
  if (step === "progress") {
    if (showAttachments) dialogTitle = "discordImport.restoreAttachments";
    else if (repairingAttachments) dialogTitle = importState === "failed"
      ? "discordImport.attachmentsStopped" : "discordImport.copyingAttachments";
    else if (importState === "failed") dialogTitle = "discordImport.historyStopped";
    else if (importState === "completed") dialogTitle = canRestoreAttachments
      ? "discordImport.serverReady" : "discordImport.completed";
  }

  useEffect(() => {
    if (showAttachments) tokenInput.current?.focus();
    else heading.current?.focus();
  }, [step, importState, showAttachments]);

  useEffect(() => () => pending.current?.abort(), []);

  useEffect(() => {
    if (step !== "progress" || importState === "completed" || importState === "failed") return;
    const controller = new AbortController();
    let timer: number | undefined;
    let misses = 0;
    const poll = async () => {
      try {
        const status = id === null
          ? await api.latest(controller.signal)
          : await api.status(id, controller.signal);
        if (controller.signal.aborted) return;
        if (status === null) {
          setStep("connect");
          return;
        }
        if (id === null) {
          rememberJob(status.id);
          setId(status.id);
        }
        setPollFailed(false);
        setJob(status);
        if (id !== null && status.state === "running") timer = window.setTimeout(() => void poll(), 1500);
      } catch (cause) {
        if (controller.signal.aborted) return;
        if (cause instanceof DiscordImportRefusal && cause.status === 404) {
          if (++misses < 4) {
            timer = window.setTimeout(() => void poll(), 1500);
            return;
          }
          rememberJob(null);
          setId(null);
          setJob(null);
          setStep("connect");
          setError("discordImport.jobMissing");
          return;
        }
        setPollFailed(true);
        timer = window.setTimeout(() => void poll(), 4000);
      }
    };
    void poll();
    return () => {
      controller.abort();
      window.clearTimeout(timer);
    };
  }, [api, id, importState, pollVersion, step]);

  const checked = preview?.channels.filter((channel) => selected.includes(channel.id)) ?? [];
  const historyChannels = checked.filter((channel) => channel.hasOwnMessages || (threads && channel.kind === "text"));
  const copiesHistory = mode === "all" || mode === "since";
  const restrictedHistory = historyChannels.filter((channel) => !channel.canCopyHistory);
  const hasRestrictedHistory = copiesHistory && restrictedHistory.length > 0;
  const cutoff = date === "" ? null : new Date(date + "T00:00:00");
  const validDate = cutoff !== null && Number.isFinite(cutoff.getTime()) && cutoff.getTime() <= openedAt;
  const selectionReady = name.trim() !== "" && checked.length > 0 &&
    (mode === "structure" || (copiesHistory && preview?.messageContent === true &&
      historyChannels.length > 0 && !hasRestrictedHistory && (mode === "all" || validDate)));

  const visible = preview?.channels.filter((channel) =>
    channel.name.toLocaleLowerCase().includes(search.trim().toLocaleLowerCase())) ?? [];
  const allVisibleSelected = visible.length > 0 && visible.every((channel) => selected.includes(channel.id));
  const groups = [
    { id: null, name: t("discordImport.uncategorized") },
    ...(preview?.sections ?? []),
  ];
  const sectionIds = new Set(preview?.sections.map((section) => section.id));

  const move = (next: Step) => {
    setError(null);
    setStep(next);
  };

  const back = () => {
    if (busy) return;
    switch (step) {
      case "connect": onBack(); break;
      case "source": move("connect"); break;
      case "channels": move("source"); break;
      case "review": move("channels"); break;
    }
  };

  const request = async (work: (signal: AbortSignal) => Promise<void>) => {
    if (pending.current !== null) return;
    const controller = new AbortController();
    pending.current = controller;
    setError(null);
    setBusy(true);
    try {
      await work(controller.signal);
    } catch (cause) {
      if (!controller.signal.aborted) {
        const key = cause instanceof DiscordImportRefusal ? ERROR_MESSAGES.get(cause.code ?? "") : undefined;
        setError(key ?? (cause instanceof DiscordImportRefusal && cause.status === 429 ? "discordImport.rateLimited" : "discordImport.requestFailed"));
      }
    } finally {
      pending.current = null;
      if (!controller.signal.aborted) setBusy(false);
    }
  };

  const track = (status: DiscordImportStatus) => {
    rememberJob(status.id);
    setId(status.id);
    setJob(status);
    setToken("");
    setShowAttachments(false);
    setPollFailed(false);
    move("progress");
  };

  const newImport = () => {
    rememberJob(null);
    setId(null);
    setJob(null);
    setToken("");
    setShowAttachments(false);
    move("connect");
  };

  const start = async (signal: AbortSignal) => {
    if (!selectionReady || mode === null) return;
    const key = id ?? crypto.randomUUID();
    const history: DiscordHistory = mode === "since" && cutoff !== null
      ? { type: "since", since: cutoff.toISOString() }
      : { type: mode === "structure" ? "structure" : "all" };
    setId(key);
    rememberJob(key);
    try {
      track(await api.start(key, token, {
        sourceId,
        name: name.trim(),
        channelIds: selected,
        history,
        includeThreads: copiesHistory && threads,
      }, signal));
    } catch (cause) {
      // With no usable reply, first look for the same job. A reload or a
      // repeated click must not create a second server.
      if (!(cause instanceof DiscordImportRefusal) || cause.status === 0 || cause.status >= 500) {
        setToken("");
        move("progress");
        return;
      }
      rememberJob(null);
      setId(null);
      throw cause;
    }
  };

  const loadSources = (refresh: boolean) => {
    void request(async (signal) => {
      const found = await api.servers(token, signal);
      setInstallUrl(found.installUrl);
      setSources(found.servers);
      setSourceId((previous) => refresh && found.servers.some((source) => source.id === previous)
        ? previous : found.servers.length === 1 ? found.servers[0]!.id : "");
      move("source");
    });
  };

  const submit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (busy) return;
    if (step === "connect" && token.trim() !== "") {
      loadSources(false);
    } else if (step === "source" && sourceId !== "") {
      void request(async (signal) => {
        const found = await api.preview(token, sourceId, signal);
        setPreview(found);
        setName(found.name.slice(0, 80).replace(/[\uD800-\uDBFF]$/, ""));
        setSelected([]);
        setSearch("");
        setMode(null);
        setDate("");
        move("channels");
      });
    } else if (step === "channels" && selectionReady) {
      move("review");
    } else if (step === "review") {
      void request(start);
    } else if (step === "progress" && job !== null && token.trim() !== ""
        && (job.state === "failed" || (canRestoreAttachments && showAttachments))) {
      void request(async (signal) => track(await api.resume(job.id, token, signal)));
    }
  };

  const tokenField = (
    <label className={styles.field}>
      {t("discordImport.token")}
      <input
        ref={tokenInput}
        className={createStyles.input}
        type="password"
        value={token}
        onChange={(event) => setToken(event.target.value)}
        autoComplete="off"
        spellCheck={false}
        maxLength={256}
      />
    </label>
  );

  const historyLabel = mode === "structure" ? t("discordImport.structure")
    : mode === "all" ? t("discordImport.allHistory") : t("discordImport.since", { date });
  const canContinue = step === "connect" ? token.trim() !== "" : step === "source" ? sourceId !== "" : selectionReady;
  const close = () => { if (!busy) onClose(); };
  const openImportedServer = () => {
    if (job?.serverId == null) return;
    rememberJob(null);
    onOpenServer(job.serverId);
  };

  return (
    <Modal open onClose={close} labelledBy={titleId} className={createStyles.wrap ?? ""}>
      <form className={styles.sheet} onSubmit={submit}>
        <header className={styles.header}>
          <h2 id={titleId} ref={heading} tabIndex={-1}>
            {t(dialogTitle)}
          </h2>
          <DialogClose label={t("common.close")} onClose={close} disabled={busy} />
        </header>

        <div className={styles.body} aria-busy={busy}>
          {step === "connect" && (
            <>
              <h3>{t("discordImport.connect")}</h3>
              <p><a href="https://discord.com/developers/applications" target="_blank" rel="noreferrer">{t("discordImport.portal")}</a></p>
              <ol className={styles.setup}>
                <li>{t("discordImport.botHelp")}</li>
                <li>{t("discordImport.botHistoryHelp")}</li>
              </ol>
              {tokenField}
            </>
          )}

          {step === "source" && (
            <>
              <h3>{t("discordImport.source")}</h3>
              {sources.length === 0 ? <p>{t("discordImport.noServers")}</p> : (
                <label className={styles.field}>
                  {t("discordImport.sourceLabel")}
                  <select className={createStyles.input} value={sourceId} onChange={(event) => setSourceId(event.target.value)}>
                    <option value="">{t("discordImport.chooseServer")}</option>
                    {sources.map((source) => <option key={source.id} value={source.id}>{source.name}</option>)}
                  </select>
                </label>
              )}
              {installUrl !== null && (
                <>
                  <p>{t("discordImport.inviteHelp")}</p>
                  <div className={styles.tools}>
                    <a href={installUrl} target="_blank" rel="noreferrer">{t("discordImport.addBot")}</a>
                    <Button type="button" variant="quiet" disabled={busy} onClick={() => loadSources(true)}>
                      {t("discordImport.refreshServers")}
                    </Button>
                  </div>
                </>
              )}
            </>
          )}

          {step === "channels" && preview !== null && (
            <>
              <h3>{t("discordImport.scope")}</h3>
              <label className={styles.field}>
                {t("shell.serverName")}
                <input className={createStyles.input} value={name} onChange={(event) => setName(event.target.value)} maxLength={80} />
              </label>
              <div className={styles.scope}>
                <fieldset className={styles.group}>
                  <legend>{t("discordImport.channels")}</legend>
                  <p>{t("discordImport.publicOnly")}</p>
                  {preview.restrictedChannels > 0 && <p>{t("discordImport.restricted", { count: preview.restrictedChannels })}</p>}
                  {preview.channels.length === 0 ? <p>{t("discordImport.noChannels")}</p> : (
                    <>
                      <div className={styles.tools}>
                        <input
                          className={createStyles.input}
                          type="search"
                          aria-label={t("discordImport.searchChannels")}
                          placeholder={t("discordImport.searchChannels")}
                          value={search}
                          onChange={(event) => setSearch(event.target.value)}
                        />
                        <Button type="button" variant="secondary" disabled={visible.length === 0} onClick={() => {
                          const visibleIds = new Set(visible.map((channel) => channel.id));
                          setSelected(allVisibleSelected
                            ? selected.filter((channel) => !visibleIds.has(channel))
                            : [...new Set([...selected, ...visibleIds])]);
                        }}>
                          {search.trim() === ""
                            ? t(allVisibleSelected ? "discordImport.clearAll" : "discordImport.selectAll")
                            : t(allVisibleSelected ? "discordImport.clearVisible" : "discordImport.selectVisible")}
                        </Button>
                      </div>
                      {visible.length === 0 && <p>{t("discordImport.noMatches")}</p>}
                      <div className={styles.channelList}>
                        {groups.map((group) => {
                          const channels = visible.filter((channel) =>
                            channel.sectionId !== null && sectionIds.has(channel.sectionId)
                              ? channel.sectionId === group.id
                              : group.id === null);
                          if (channels.length === 0) return null;
                          return (
                            <fieldset className={styles.category} key={group.id}>
                              <legend>{group.name}</legend>
                              {channels.map((channel) => (
                                <label className={styles.option} key={channel.id}>
                                  <input type="checkbox" checked={selected.includes(channel.id)} onChange={(event) => {
                                    setSelected(event.target.checked
                                      ? [...selected, channel.id]
                                      : selected.filter((chosen) => chosen !== channel.id));
                                  }} />
                                  <Icon name={channel.kind === "voice" ? "volumeLow" : "hash"} size={16} />
                                  <span>{channel.name}</span>
                                </label>
                              ))}
                            </fieldset>
                          );
                        })}
                      </div>
                      <p>{t("discordImport.selected", { count: checked.length })}</p>
                    </>
                  )}
                </fieldset>
                <fieldset className={styles.group}>
                  <legend>{t("discordImport.history")}</legend>
                  {mode === null && <p>{t("discordImport.chooseHistory")}</p>}
                  {(["structure", "all", "since"] as const).map((value) => (
                    <label className={styles.option} key={value}>
                      <input type="radio" name="history" value={value} checked={mode === value}
                        disabled={value !== "structure" && !preview.messageContent} onChange={() => setMode(value)} />
                      {t(value === "structure" ? "discordImport.structure" : value === "all" ? "discordImport.allHistory" : "discordImport.sinceDate")}
                    </label>
                  ))}
                  {!preview.messageContent && (
                    <>
                      <p>{t("discordImport.messageContent")}</p>
                      <Button type="button" variant="secondary" disabled={busy} onClick={() => {
                        void request(async (signal) => {
                          const found = await api.preview(token, sourceId, signal);
                          const available = new Set(found.channels.map((channel) => channel.id));
                          setPreview(found);
                          setSelected((current) => current.filter((channel) => available.has(channel)));
                        });
                      }}>{t("discordImport.refreshHistoryAccess")}</Button>
                    </>
                  )}
                  {mode === "since" && (
                    <label className={styles.field}>
                      {t("discordImport.startDate")}
                      <input className={createStyles.input} type="date" value={date} onChange={(event) => setDate(event.target.value)} />
                    </label>
                  )}
                  {copiesHistory && (
                    <label className={styles.option}>
                      <input type="checkbox" checked={threads} onChange={(event) => setThreads(event.target.checked)} />
                      {t("discordImport.threads")}
                    </label>
                  )}
                  {copiesHistory && historyChannels.length === 0 && <p>{t("discordImport.noHistory")}</p>}
                  {hasRestrictedHistory && <p>{t("discordImport.historyRestrictedChannels", {
                    channels: restrictedHistory.map((channel) => `#${channel.name}`).join(", "),
                  })}</p>}
                </fieldset>
              </div>
            </>
          )}

          {step === "review" && preview !== null && (
            <>
              <h3>{t("discordImport.review")}</h3>
              <dl className={styles.summary}>
                <dt>{t("discordImport.sourceLabel")}</dt><dd>{preview.name}</dd>
                <dt>{t("shell.serverName")}</dt><dd>{name.trim()}</dd>
                <dt>{t("discordImport.channels")}</dt><dd>{checked.map((channel) => channel.name).join(", ")}</dd>
                <dt>{t("discordImport.history")}</dt><dd>{historyLabel}</dd>
                {copiesHistory && threads && <><dt>{t("discordImport.reviewThreads")}</dt><dd>{t("discordImport.included")}</dd></>}
              </dl>
              <div className={styles.notice}>
                <p>{t("discordImport.reviewHelp")}</p>
                {copiesHistory && <p>{t("discordImport.reviewAttachments")}</p>}
              </div>
            </>
          )}

          {step === "progress" && (
            <>
              <h3>{job?.name ?? t("discordImport.waiting")}</h3>
              {showAttachments ? (
                <>
                  <p>{t("discordImport.attachmentsHelp")}</p>
                  {tokenField}
                </>
              ) : importState === "completed" ? (
                job && (
                  <>
                    <dl className={styles.summary}>
                      <dt>{t("discordImport.channelsImported")}</dt><dd>{number.format(job.channelsImported)}</dd>
                      <dt>{t("discordImport.sectionsImported")}</dt><dd>{number.format(job.sectionsImported)}</dd>
                      {job.historyRequested && job.report !== null && (
                        <>
                          <dt>{t("discordImport.messagesCopied")}</dt><dd>{number.format(job.report.messages)}</dd>
                          {job.report.skippedMessages > 0 && (
                            <><dt>{t("discordImport.messagesSkipped")}</dt><dd>{number.format(job.report.skippedMessages)}</dd></>
                          )}
                        </>
                      )}
                    </dl>
                    {!job.historyRequested && <p>{t("discordImport.structureResult")}</p>}
                    {job.historyRequested && job.report?.messages === 0 && job.report.skippedMessages === 0 && (
                      <p>{t("discordImport.emptyHistoryResult")}</p>
                    )}
                    {canRestoreAttachments ? (
                      <section className={styles.notice} aria-labelledby={attachmentsTitleId}>
                        <h4 id={attachmentsTitleId}>{t("discordImport.attachmentsPending")}</h4>
                        <dl className={styles.summary}>
                          <dt>{t("discordImport.messagesMissingAttachments")}</dt><dd>{number.format(pendingAttachments)}</dd>
                        </dl>
                        <p>{t("discordImport.attachmentOnlyHelp")}</p>
                        <p>{t("discordImport.keepBot")}</p>
                      </section>
                    ) : <p>{t("discordImport.removeBot")}</p>}
                  </>
                )
              ) : importState === "failed" ? (
                <>
                  <p role="alert">{t(ERROR_MESSAGES.get(job?.error ?? "") ?? "discordImport.requestFailed")}</p>
                  <p>{t("discordImport.resumeHelp")}</p>
                  {tokenField}
                </>
              ) : (
                <>
                  {repairingAttachments ? (
                    <p role="status">{t("discordImport.attachmentsProgress", { remaining: number.format(pendingAttachments) })}</p>
                  ) : (
                    <>
                      <p role="status">{t(job?.totalSources == null ? "discordImport.preparing"
                        : job.totalSources > 0 ? "discordImport.progress" : "discordImport.finishing",
                      { current: job?.completedSources ?? 0, total: job?.totalSources ?? 0 })}</p>
                      <progress aria-label={t("discordImport.title")}
                        max={job?.totalSources || undefined} value={job?.totalSources ? job.completedSources : undefined} />
                    </>
                  )}
                  <p>{t("discordImport.progressHelp")}</p>
                  {pollFailed && <p role="alert">{t("discordImport.pollFailed")}</p>}
                </>
              )}
            </>
          )}

          {error !== null && <p className={styles.error} role="alert">{t(error)}</p>}
        </div>

        <footer className={styles.footer}>
          {step !== "progress" ? (
            <>
              <Button type="button" variant="quiet" onClick={back} disabled={busy}>{t("discordImport.back")}</Button>
              <Button type="submit" variant="primary" disabled={busy || !canContinue}>
                {t(busy ? "discordImport.waiting" : step === "review" ? "discordImport.start"
                  : step === "connect" ? "discordImport.connectButton" : "shell.createServerNext")}
              </Button>
            </>
          ) : importState === "completed" ? (
            <>
              {showAttachments ? (
                <>
                  <Button type="button" variant="quiet" disabled={busy} onClick={() => {
                    setShowAttachments(false);
                    setToken("");
                  }}>{t("discordImport.back")}</Button>
                  <Button type="submit" variant="primary" disabled={busy || token.trim() === ""}>
                    {t(busy ? "discordImport.waiting" : "discordImport.startAttachments")}
                  </Button>
                </>
              ) : (
                <>
                  <Button type="button" variant="quiet" onClick={newImport}>{t("discordImport.newImport")}</Button>
                  <Button type="button" variant={canRestoreAttachments ? "secondary" : "primary"}
                    disabled={job?.serverId == null} onClick={openImportedServer}>
                    {t("discordImport.openServer")}
                  </Button>
                  {canRestoreAttachments && (
                    <Button type="button" variant="primary" onClick={() => setShowAttachments(true)}>
                      {t("discordImport.restoreAttachments")}
                    </Button>
                  )}
                </>
              )}
            </>
          ) : importState === "failed" ? (
            <>
              {!repairingAttachments && <Button type="button" variant="quiet" disabled={busy} onClick={() => {
                if (job === null) return;
                void request(async (signal) => {
                  await api.discard(job.id, signal);
                  newImport();
                });
              }}>{t("discordImport.discard")}</Button>}
              {repairingAttachments && job?.serverId != null &&
                <Button type="button" variant="secondary" disabled={busy} onClick={openImportedServer}>{t("discordImport.openServer")}</Button>}
              <Button type="submit" variant="primary" disabled={busy || token.trim() === ""}>
                {t(busy ? "discordImport.waiting" : repairingAttachments ? "discordImport.resumeAttachments" : "discordImport.resume")}
              </Button>
            </>
          ) : (
            <>
              {pollFailed && <Button type="button" variant="quiet" onClick={() => setPollVersion((version) => version + 1)}>{t("common.retry")}</Button>}
              {repairingAttachments && job?.serverId != null &&
                <Button type="button" variant="quiet" onClick={openImportedServer}>{t("discordImport.openServer")}</Button>}
              <Button type="button" variant="secondary" onClick={close}>{t("common.close")}</Button>
            </>
          )}
        </footer>
      </form>
    </Modal>
  );
}
