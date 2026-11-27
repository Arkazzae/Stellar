import { useState } from "react";

import { Button } from "@stellarr/ui";
import { Icon } from "@stellarr/icons";
import { Avatar } from "@/components/primitives/Avatar";
import { useConfirm } from "@/components/primitives/confirmContext";
import { ContextMenu, useContextMenu } from "@/components/primitives/ContextMenu";
import type { MessageKey } from "@/i18n";
import {
  calendarLeaf,
  eventAction,
  eventPhase,
  eventStatus,
  eventWhen,
  placeLink,
  placeName,
  type EventChannelNow,
} from "@/lib/events/eventCard";
import { EventRefusedError, type EventRefusalReason, type ServerEvent } from "@/ports";
import { useLanguage } from "@/state/hooks";

import css from "./EventCard.module.css";

/** What a card can ask for; without them it only shows. */
export interface EventCardActions {
  /** Signs up, or joins the waitlist when it is full. */
  signUp?(event: ServerEvent): Promise<void>;
  /** Signs out, or leaves the waitlist. */
  signOut?(event: ServerEvent): Promise<void>;
  /** Enters the event's voice channel, as the rail does. */
  join?(channelId: string): void;
}

/** What only its creator and those who manage events may do. */
export interface EventCardManagement {
  edit(event: ServerEvent): void;
  cancel(event: ServerEvent): Promise<void>;
}

export interface EventCardProps {
  event: ServerEvent;
  now: Date;
  /** The reader's zone, which the times are shown in. */
  zone: string;
  /** What is known of its voice channel right now. */
  channel?: EventChannelNow;
  /** The picture of someone signed up, when it is known. */
  faceOf?: (userId: string) => string | undefined;
  actions?: EventCardActions;
  manage?: EventCardManagement;
}

const REFUSALS: Record<EventRefusalReason, MessageKey> = {
  over: "event.refused.over",
  live: "event.refused.live",
  moved: "event.refused.moved",
  gone: "event.refused.gone",
  forbidden: "event.refused.forbidden",
  limit: "event.refused.limit",
  other: "event.refused.other",
};

/**
 * An event, as its announcement, the list and a link show it: the date on a
 * calendar leaf, its name, when and where, and one button that says what
 * pressing it does. Seats count sign-ups only; while it runs anyone who may
 * enter its channel is offered to.
 */
export function EventCard({ event, now, zone, channel = {}, faceOf, actions, manage }: EventCardProps) {
  const { t, locale } = useLanguage();
  const confirm = useConfirm();
  const menu = useContextMenu();
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState<MessageKey | null>(null);

  const phase = eventPhase(event, now);
  const action = eventAction(event, now, channel);
  const leaf = calendarLeaf(new Date(event.startsAt), zone, locale);
  const link = placeLink(event);
  const where = placeName(event, channel);

  const run = async (deed: (() => Promise<void>) | undefined) => {
    if (deed === undefined) return;
    setBusy(true);
    setProblem(null);
    try {
      await deed();
    } catch (error) {
      setProblem(REFUSALS[error instanceof EventRefusedError ? error.reason : "other"]);
    } finally {
      setBusy(false);
    }
  };
  const signUp = actions?.signUp;
  const signOut = actions?.signOut;

  const button = (() => {
    switch (action.kind) {
      case "signUp":
        return signUp === undefined ? null : (
          <Button type="button" variant="primary" disabled={busy} onClick={() => void run(() => signUp(event))}>
            {t("event.signUp")}
          </Button>
        );
      case "signedUp":
        return signOut === undefined ? null : (
          <Button
            type="button"
            variant="secondary"
            className={css.signed}
            aria-pressed
            title={t("event.signOut")}
            disabled={busy}
            onClick={() => void run(() => signOut(event))}
          >
            <Icon name="check" size={15} />
            {t("event.signedUpButton")}
          </Button>
        );
      case "joinWaitlist":
        return signUp === undefined ? null : (
          <Button type="button" variant="secondary" disabled={busy} onClick={() => void run(() => signUp(event))}>
            {t("event.joinWaitlist")}
          </Button>
        );
      case "onWaitlist":
        return signOut === undefined ? null : (
          <Button
            type="button"
            variant="secondary"
            aria-pressed
            title={t("event.leaveWaitlist")}
            disabled={busy}
            onClick={() => void run(() => signOut(event))}
          >
            {t("event.onWaitlist")}
          </Button>
        );
      case "join": {
        const join = actions?.join;
        return join === undefined ? null : (
          <Button type="button" variant="primary" onClick={() => join(action.channelId)}>
            {t("event.join", { channel: action.channelName })}
          </Button>
        );
      }
      case "none":
        return null;
    }
  })();

  return (
    <article
      className={[css.card, phase === "live" && css.live, (phase === "ended" || phase === "cancelled") && css.ended]
        .filter(Boolean)
        .join(" ")}
      aria-label={event.name}
    >
      <div className={css.top}>
        <span className={css.leaf} aria-hidden="true">
          <i>{leaf.weekday}</i>
          <b>{leaf.day}</b>
        </span>
        <div className={css.text}>
          <h4 className={css.name}>
            {event.name}
            {phase === "live" ? <span className={css.livePill}>{t("event.live")}</span> : null}
            {phase === "ended" ? <span className={css.endedPill}>{t("event.ended")}</span> : null}
            {phase === "cancelled" ? <span className={css.endedPill}>{t("event.cancelled")}</span> : null}
          </h4>
          <span className={css.when}>{eventWhen(event, now, zone, locale, t, channel)}</span>
          <span className={css.where}>
            <Icon name={event.place.kind === "voice" ? "speaker" : "globe"} size={14} />
            {link === undefined ? (
              where
            ) : (
              <a href={link} target="_blank" rel="noopener noreferrer">
                {where}
              </a>
            )}
          </span>
        </div>
        {manage === undefined || phase === "ended" || phase === "cancelled" ? null : (
          <button
            type="button"
            className={css.more}
            aria-label={t("event.manage")}
            aria-haspopup="menu"
            onClick={(click) => {
              const box = click.currentTarget.getBoundingClientRect();
              menu.openAt(click.currentTarget, box.right, box.bottom);
            }}
          >
            <Icon name="dots" size={16} />
          </button>
        )}
      </div>
      {event.description === undefined || phase === "live" ? null : (
        <p className={css.description}>{event.description}</p>
      )}
      {/* A cancelled one has nobody to show, nothing to count and nothing to press. */}
      {phase === "cancelled" ? null : (
        <div className={css.foot}>
          {event.faces.length === 0 ? null : (
            <span className={css.faces} aria-hidden="true">
              {event.faces.slice(0, 3).map((person) => {
                const face = faceOf?.(person.id);
                return (
                  <Avatar
                    key={person.id}
                    id={person.id}
                    displayName={person.displayName}
                    size="xs"
                    {...(face === undefined ? {} : { avatarUrl: face })}
                  />
                );
              })}
            </span>
          )}
          <span className={css.status}>{eventStatus(event, now, locale, t, channel)}</span>
          <span className={css.buttons}>{button}</span>
        </div>
      )}
      {problem === null ? null : (
        <p className={css.problem} role="alert">
          {t(problem)}
        </p>
      )}
      {menu.at === null || manage === undefined ? null : (
        <ContextMenu
          at={menu.at}
          label={t("event.manage")}
          entries={[
            { id: "edit", label: t("event.edit"), icon: "edit", onSelect: () => manage.edit(event) },
            {
              id: "cancel",
              label: t("event.cancel"),
              icon: "trash",
              danger: true,
              onSelect: () =>
                void confirm({
                  title: t("event.cancelTitle"),
                  description: t("event.cancelDescription"),
                  confirmLabel: t("event.cancel"),
                  busyLabel: t("event.cancelling"),
                  failure: t("event.refused.other"),
                  danger: true,
                  run: () => manage.cancel(event),
                }),
            },
          ]}
          onClose={menu.close}
        />
      )}
    </article>
  );
}
