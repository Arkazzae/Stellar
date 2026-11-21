import { Fragment, useEffect, useId, useState, type CSSProperties, type ReactNode } from "react";

import { Button, Dropdown } from "@stellarr/ui";
import { Icon } from "@stellarr/icons";
import { Avatar } from "@/components/primitives/Avatar";
import { countedNoun, formatShortDate } from "@/components/settings/paneFormat";
import { SettingsSection } from "@/components/settings/SettingsSection";
import { PERMISSION_LABEL } from "@/components/settings/server/panes/RolesPane";
import { useAsyncAction } from "@/hooks/useAsyncAction";
import type { InterfaceLocale, MessageKey, Translator } from "@/i18n";
import { formatTime, isSameDay } from "@/lib/format/format";
import type { AuditChange, AuditEntry, ServerPermission } from "@/ports";
import { useLanguage } from "@/state/hooks";

/** How many entries the server sends at a time. */
export const AUDIT_PAGE = 50;

type Kind = "server" | "channels" | "roles" | "members" | "moderation" | "invites" | "emoji" | "soundboard" | "operator";

const KINDS: readonly Kind[] = ["server", "channels", "roles", "members", "moderation", "invites", "emoji", "soundboard", "operator"];

/** Which filter an action falls under; an entry without one (an older line) falls under none. */
function kindOf(action: string | undefined): Kind | null {
  if (action === undefined) return null;
  if (action === "operator") return "operator";
  if (action === "member.role") return "members";
  if (action.startsWith("member.") || action.startsWith("voice.")) return "moderation";
  if (action.startsWith("channel.") || action.startsWith("section.")) return "channels";
  if (action.startsWith("role.")) return "roles";
  if (action.startsWith("invite.")) return "invites";
  if (action.startsWith("emoji.")) return "emoji";
  if (action.startsWith("sound")) return "soundboard";
  if (action.startsWith("server.")) return "server";
  return null;
}

/** The sentence each action is said with, by its action and, for visibility, what it became. */
function sentenceKey(entry: AuditEntry): MessageKey | null {
  switch (entry.action) {
    case "server.update": return "srv.audit.a.server.update";
    case "server.visibility":
      return entry.changes?.[0]?.to === "public" ? "srv.audit.a.server.public" : "srv.audit.a.server.inviteOnly";
    case "server.icon": return "srv.audit.a.server.icon";
    case "server.icon.remove": return "srv.audit.a.server.iconRemove";
    case "server.transfer": return "srv.audit.a.server.transfer";
    case "channel.create": return "srv.audit.a.channel.create";
    case "channel.update": return "srv.audit.a.channel.update";
    case "channel.delete": return "srv.audit.a.channel.delete";
    case "channel.move": return "srv.audit.a.channel.move";
    case "channel.reorder": return "srv.audit.a.channel.reorder";
    case "channel.permissions": return "srv.audit.a.channel.permissions";
    case "section.create": return "srv.audit.a.section.create";
    case "section.update": return "srv.audit.a.section.update";
    case "section.delete": return "srv.audit.a.section.delete";
    case "section.reorder": return "srv.audit.a.section.reorder";
    case "section.permissions": return "srv.audit.a.section.permissions";
    case "role.create": return "srv.audit.a.role.create";
    case "role.update": return "srv.audit.a.role.update";
    case "role.delete": return "srv.audit.a.role.delete";
    case "member.role": return "srv.audit.a.member.role";
    case "member.kick": return "srv.audit.a.member.kick";
    case "member.ban": return "srv.audit.a.member.ban";
    case "member.unban": return "srv.audit.a.member.unban";
    case "voice.disconnect": return "srv.audit.a.voice.disconnect";
    case "voice.move": return "srv.audit.a.voice.move";
    case "invite.create": return "srv.audit.a.invite.create";
    case "invite.revoke": return "srv.audit.a.invite.revoke";
    case "emoji.create": return "srv.audit.a.emoji.create";
    case "emoji.delete": return "srv.audit.a.emoji.delete";
    case "sound.create": return "srv.audit.a.sound.create";
    case "sound.update": return "srv.audit.a.sound.update";
    case "sound.delete": return "srv.audit.a.sound.delete";
    case "soundboard.settings": return "srv.audit.a.soundboard.settings";
    default: return null;
  }
}

const FIELD_LABEL: Record<string, MessageKey> = {
  name: "srv.audit.f.name",
  description: "srv.audit.f.description",
  visibility: "srv.audit.f.visibility",
  color: "srv.audit.f.color",
  permissions: "srv.audit.f.permissions",
  allow: "srv.audit.f.allow",
  deny: "srv.audit.f.deny",
  role: "srv.audit.f.role",
  section: "srv.audit.f.section",
  reason: "srv.audit.f.reason",
  topic: "srv.audit.f.topic",
  slowMode: "srv.audit.f.slowMode",
  hideAfter: "srv.audit.f.hideAfter",
  userLimit: "srv.audit.f.userLimit",
  kind: "srv.audit.f.kind",
  channel: "srv.audit.f.channel",
  holders: "srv.audit.f.holders",
  everyoneCanPlay: "srv.audit.f.everyoneCanPlay",
  rateLimited: "srv.audit.f.rateLimited",
  glyph: "srv.audit.f.glyph",
  trim: "srv.audit.f.trim",
};

const fieldLabel = (t: Translator, field: string) =>
  FIELD_LABEL[field] === undefined ? field : t(FIELD_LABEL[field]);

/** A placeholder that cannot occur in a name, so the sentence can be cut around it. */
const SLOT = "\u0000";

/** The sentence with the actor and the target in bold, in the reader's language. */
function sentence(entry: AuditEntry, t: Translator): { plain: boolean; node: ReactNode } {
  const key = sentenceKey(entry);
  if (key === null) return { plain: true, node: entry.summary };
  const actor = entry.actor?.displayName ?? t("srv.audit.unknownActor");
  const name = entry.target?.name ?? "";
  const target =
    entry.target?.type === "channel"
      ? `#${name}`
      : entry.target?.type === "emoji"
        ? `:${name}:`
        : name;
  const text = t(key, { actor: `${SLOT}a${SLOT}`, target: `${SLOT}t${SLOT}` });
  const node = text.split(SLOT).map((part, index) => {
    if (part === "a") return <strong key={index}>{actor}</strong>;
    if (part === "t") return <strong key={index}>{target}</strong>;
    return <Fragment key={index}>{part}</Fragment>;
  });
  return { plain: false, node };
}

function permissionName(t: Translator, name: string): string {
  const key = PERMISSION_LABEL[name as ServerPermission] as MessageKey | undefined;
  return key === undefined ? name : t(key);
}

/** One value of a change, worded: tokens as words, a role as its pill, a colour as its swatch. */
function value(change: AuditChange, side: "from" | "to", t: Translator): ReactNode {
  const raw = change[side];
  const color = side === "from" ? change.fromColor : change.toColor;
  if (raw === undefined || raw === null) {
    return change.field === "section" ? t("srv.audit.v.noSection") : t("srv.audit.v.empty");
  }
  if (change.field === "role") {
    return (
      <span className="ss-role-pill">
        <span className="ss-role-ico" style={{ "--rc": color ?? "var(--text-faint)" } as CSSProperties} aria-hidden="true">
          <Icon name="shield" size={14} />
        </span>
        <span className="ss-pill-label">{raw}</span>
      </span>
    );
  }
  if (change.field === "color") {
    return (
      <>
        <span className="ss-sw" style={{ "--sw": color ?? raw } as CSSProperties} aria-hidden="true" />
        {raw}
      </>
    );
  }
  if (change.field === "visibility") return raw === "public" ? t("srv.audit.v.public") : t("srv.audit.v.inviteOnly");
  if (change.field === "kind") return raw === "voice" ? t("channel.voice") : t("channel.text");
  if (change.field === "everyoneCanPlay" || change.field === "rateLimited") {
    return raw === "on" ? t("srv.audit.v.on") : t("srv.audit.v.off");
  }
  if (raw === "") return t("srv.audit.v.empty");
  return raw;
}

function changeValue(change: AuditChange, t: Translator, locale: InterfaceLocale): ReactNode {
  if (change.added !== undefined || change.removed !== undefined) {
    return (
      <>
        {(change.added ?? []).map((name) => (
          <span className="ss-chip ss-plus" key={`+${name}`}>
            + {permissionName(t, name)}
          </span>
        ))}
        {(change.removed ?? []).map((name) => (
          <span className="ss-chip ss-minus" key={`-${name}`}>
            − {permissionName(t, name)}
          </span>
        ))}
      </>
    );
  }
  if (change.field === "holders" && change.to !== undefined) {
    return t("srv.audit.v.holders", { people: countedNoun(t, locale, "settings.member", Number(change.to)) });
  }
  if (change.from === undefined) return value(change, "to", t);
  return (
    <>
      <span className="ss-was">{value(change, "from", t)}</span>
      <span className="ss-arrow" aria-label={t("srv.audit.changedTo")}>
        →
      </span>
      <span className="ss-to">{value(change, "to", t)}</span>
    </>
  );
}

/** "Today", "Yesterday", or the date. */
function dayOf(iso: string, now: Date, t: Translator, locale: InterfaceLocale): string {
  const at = new Date(iso);
  if (Number.isNaN(at.getTime())) return "";
  if (isSameDay(at, now)) return t("srv.audit.today");
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (isSameDay(at, yesterday)) return t("srv.audit.yesterday");
  return formatShortDate(iso, locale);
}

export interface AuditPaneProps {
  supported: boolean;
  /** The newest page. */
  entries: readonly AuditEntry[];
  /** A page of entries older than `before`; absent where the log cannot page. */
  onLoadOlder?: (before: string) => Promise<AuditEntry[]>;
  /**
   * Asks for the newest page again. The log is read when the dialog opens,
   * and what was done since, in this dialog or elsewhere, belongs in it.
   */
  onShow?: () => void;
}

/**
 * Who changed what, newest first, a card per day. A line says it in the
 * reader's language with the person who did it and what they did it to, and
 * opens to what changed. Older lines, written before the server kept their
 * parts, are shown as the sentence they were written as.
 */
export function AuditPane({ supported, entries, onLoadOlder, onShow }: AuditPaneProps) {
  const { locale, t } = useLanguage();
  // Read once per mount so every row agrees on what "today" is.
  const [now] = useState(() => new Date());
  const [older, setOlder] = useState<AuditEntry[]>([]);
  const [ended, setEnded] = useState(false);
  const [open, setOpen] = useState<ReadonlySet<string>>(new Set());
  const [who, setWho] = useState("");
  const [what, setWhat] = useState("");
  const loading = useAsyncAction();
  const baseId = useId();

  // Once per opening of the pane, not per render: the reload hands new
  // entries in, which would ask again.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => onShow?.(), []);

  const all = [...entries, ...older];
  const atEnd = ended || entries.length < AUDIT_PAGE || onLoadOlder === undefined;

  const actors = new Map<string, string>();
  for (const entry of all) {
    if (entry.actor !== undefined) actors.set(entry.actor.id, entry.actor.displayName);
  }
  const kinds = KINDS.filter((kind) => all.some((entry) => kindOf(entry.action) === kind));
  const shown = all.filter(
    (entry) =>
      (who === "" || entry.actor?.id === who) && (what === "" || kindOf(entry.action) === what),
  );

  const days: { label: string; entries: AuditEntry[] }[] = [];
  for (const entry of shown) {
    const label = dayOf(entry.at, now, t, locale);
    const last = days[days.length - 1];
    if (last?.label === label) last.entries.push(entry);
    else days.push({ label, entries: [entry] });
  }

  const loadOlder = async () => {
    const last = all[all.length - 1];
    if (onLoadOlder === undefined || last === undefined) return;
    await loading.run(async () => {
      const page = await onLoadOlder(last.at);
      setOlder((current) => [...current, ...page]);
      if (page.length < AUDIT_PAGE) setEnded(true);
    });
  };

  const toggle = (id: string) =>
    setOpen((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const filtered = who !== "" || what !== "";

  return (
    <>
      {supported ? null : <p className="set-note">{t("settings.auditUnsupported")}</p>}

      {all.length === 0 ? null : (
        <div className="ss-log-filters" role="group" aria-label={t("srv.nav.audit")}>
          <Dropdown
            options={[
              { value: "", label: t("srv.audit.everyone") },
              ...[...actors].sort((a, b) => a[1].localeCompare(b[1], locale)).map(([id, name]) => ({ value: id, label: name })),
            ]}
            value={who}
            onChange={setWho}
            label={t("srv.audit.who")}
          />
          <Dropdown
            options={[
              { value: "", label: t("srv.audit.all") },
              ...kinds.map((kind) => ({ value: kind, label: t(`srv.audit.kind.${kind}`) })),
            ]}
            value={what}
            onChange={setWhat}
            label={t("srv.audit.what")}
          />
        </div>
      )}

      {all.length === 0 ? (
        <SettingsSection title={t("srv.nav.audit")} bodyClassName="sec-list">
          <p className="acc-security-item acc-security-empty">{t("srv.audit.empty")}</p>
        </SettingsSection>
      ) : shown.length === 0 ? (
        <SettingsSection title={t("srv.nav.audit")} bodyClassName="sec-list ss-list">
          <p className="acc-security-item acc-security-empty">{t("srv.audit.none")}</p>
          <div className="sec-row ss-foot">
            <span className="ss-foot-note" />
            <Button
              type="button"
              variant="secondary"
              size="sm"
              onClick={() => {
                setWho("");
                setWhat("");
              }}
            >
              {t("srv.audit.clear")}
            </Button>
          </div>
        </SettingsSection>
      ) : (
        days.map((day) => (
          <SettingsSection key={day.label} title={day.label} bodyClassName="sec-list ss-list">
            {day.entries.map((entry) => {
              const said = sentence(entry, t);
              const changes = entry.changes ?? [];
              const detailId = `${baseId}-${entry.id}`;
              const expanded = open.has(entry.id);
              const meta = [
                formatTime(entry.at, locale),
                ...(said.plain && entry.actor !== undefined && !entry.summary.startsWith(entry.actor.displayName)
                  ? [entry.actor.displayName]
                  : []),
                ...(changes.length > 0 ? [changes.map((change) => fieldLabel(t, change.field)).join(", ")] : []),
              ].join(" · ");
              const face =
                entry.actor === undefined ? (
                  <span className="av av-sm" aria-hidden="true" />
                ) : (
                  <Avatar
                    id={entry.actor.id}
                    displayName={entry.actor.displayName}
                    avatarUrl={entry.actor.avatarUrl}
                    size="sm"
                  />
                );
              const copy = (
                <span className="sec-copy">
                  <b className={said.plain ? "ss-log-line plain" : "ss-log-line"}>{said.node}</b>
                  <small>{meta}</small>
                </span>
              );
              return (
                <div className="ss-log-item" key={entry.id}>
                  {changes.length === 0 ? (
                    <div className="sec-row ss-log">
                      {face}
                      {copy}
                    </div>
                  ) : (
                    <>
                      <button
                        type="button"
                        className="sec-row ss-log"
                        aria-expanded={expanded}
                        aria-controls={detailId}
                        onClick={() => toggle(entry.id)}
                      >
                        {face}
                        {copy}
                        <span className="ss-log-chev" aria-hidden="true">
                          <Icon name="caret" size={16} />
                        </span>
                      </button>
                      <dl className="ss-log-detail" id={detailId} hidden={!expanded}>
                        {changes.map((change, index) => (
                          <Fragment key={`${change.field}-${index}`}>
                            <dt>{fieldLabel(t, change.field)}</dt>
                            <dd>{changeValue(change, t, locale)}</dd>
                          </Fragment>
                        ))}
                      </dl>
                    </>
                  )}
                </div>
              );
            })}
          </SettingsSection>
        ))
      )}

      {all.length === 0 || filtered && shown.length === 0 ? null : (
        <p className="ss-log-end">
          {atEnd ? (
            t("srv.audit.end")
          ) : (
            <Button type="button" variant="secondary" size="sm" disabled={loading.pending} onClick={() => void loadOlder()}>
              {t("srv.audit.older")}
            </Button>
          )}
        </p>
      )}
      {loading.failed ? (
        <p className="set-note sound-error" role="alert">
          {t("srv.audit.olderFailed")}
        </p>
      ) : null}
    </>
  );
}
