import { Button } from "@stellarr/ui";
import { Icon, type IconName } from "@stellarr/icons";
import type { TranscriptNotice } from "@/components/room/transcript/MessageGroup";
import { pluralForm } from "@/lib/format/format";
import type { TimelineLineKind } from "@/ports";
import { useLanguage } from "@/state/LanguageProvider";

import { lineText, sealedCountKey } from "./encryptionCopy";

/** What a conversation's notices need to know beyond the entry itself. */
export interface DmNoticeContext {
  selfId: string;
  peerName: string;
  /** The request line still waiting for an answer. */
  liveRequestId: string | null;
  declinedIds: ReadonlySet<string>;
  dismissedRequests: ReadonlySet<string>;
  encryptionOn: boolean;
  /** A PIN would open what is sealed here: a backup exists and this browser can decrypt. */
  canEnterPin: boolean;
  /** Why this browser cannot decrypt, when it cannot. */
  unsupportedReason: string | null;
  busy: boolean;
  onCancelRequest(): void;
  onConfirmRequest(): void;
  onDismissRequest(lineId: string): void;
  onCompareCode(): void;
  onEnterPin(): void;
}

const LINE_LOOK: Record<TimelineLineKind, { icon: IconName; tone: "" | "ok" | "warn" }> = {
  encryption_requested: { icon: "lock", tone: "" },
  encryption_request_cancelled: { icon: "lock", tone: "" },
  encryption_enabled: { icon: "lock", tone: "ok" },
  encryption_disabled: { icon: "unlock", tone: "" },
  device_added: { icon: "mobile", tone: "" },
  keys_reset: { icon: "key", tone: "warn" },
};

/**
 * One entry of a conversation's history that is not a message: a line about
 * the encryption switch or someone's keys, the other person's request as a
 * card with its answers, or messages this browser cannot read, counted.
 */
export function DmNotice({
  notice,
  lineId,
  authorId,
  context,
}: {
  notice: TranscriptNotice;
  /** The entry's message id. */
  lineId: string;
  authorId: string;
  context: DmNoticeContext;
}) {
  const { t, locale } = useLanguage();

  if (notice.kind === "sealed") {
    const reason = notice.reason;
    const detail =
      reason === "unsupported"
        ? (context.unsupportedReason ?? t("e2ee.switch.unsupported"))
        : reason === "pin"
          ? context.canEnterPin
            ? t("e2ee.sealed.pin")
            : t("e2ee.sealed.noBackup")
          : t("e2ee.sealed.lost");
    return (
      <div className="dm-sealed" role="note">
        <span className="dm-notice-ico" aria-hidden="true">
          <Icon name="lock" size={18} />
        </span>
        <span className="dm-sealed-txt">
          <b>{t(sealedCountKey(pluralForm(locale, notice.count)), { count: notice.count })}</b>
          {detail}
        </span>
        {reason === "pin" && context.canEnterPin ? (
          <Button type="button" variant="secondary" size="sm" onClick={context.onEnterPin}>
            {t("e2ee.sealed.enterPin")}
          </Button>
        ) : null}
      </div>
    );
  }

  const { line, actorName } = notice;
  const self = authorId === context.selfId;
  const live = context.liveRequestId === lineId;

  if (line.kind === "encryption_requested" && live && !self && !context.dismissedRequests.has(lineId)) {
    return (
      <div className="dm-req" role="group" aria-label={t("e2ee.card.title", { name: actorName })}>
        <span className="dm-notice-ico" aria-hidden="true">
          <Icon name="lock" size={18} />
        </span>
        <div className="dm-req-txt">
          <b>{t("e2ee.card.title", { name: actorName })}</b>
          <p>{t("e2ee.card.body")}</p>
          {context.unsupportedReason === null ? (
            <div className="dm-req-acts">
              <Button type="button" variant="primary" size="sm" disabled={context.busy} onClick={context.onConfirmRequest}>
                {t("e2ee.card.confirm")}
              </Button>
              <Button type="button" variant="secondary" size="sm" onClick={() => context.onDismissRequest(lineId)}>
                {t("e2ee.card.later")}
              </Button>
            </div>
          ) : (
            <p className="dm-req-why">{context.unsupportedReason}</p>
          )}
        </div>
      </div>
    );
  }

  const look = LINE_LOOK[line.kind];
  const text = lineText(t, line, actorName, {
    self,
    live,
    declined: context.declinedIds.has(lineId),
    peerName: context.peerName,
  });
  let action: { label: string; run: () => void } | null = null;
  if (line.kind === "encryption_requested" && live) {
    action = self
      ? { label: t("e2ee.line.cancelRequest"), run: context.onCancelRequest }
      : context.unsupportedReason === null
        ? { label: t("e2ee.line.confirm"), run: context.onConfirmRequest }
        : null;
  } else if (line.kind === "keys_reset" && context.encryptionOn) {
    action = { label: t("e2ee.line.compare"), run: context.onCompareCode };
  }

  return (
    <div className={look.tone === "" ? "dm-sys" : `dm-sys ${look.tone}`} role="note">
      <span className="dm-notice-ico" aria-hidden="true">
        <Icon name={look.icon} size={16} />
      </span>
      <span className="dm-sys-txt">
        {text}
        {action === null ? null : (
          <>
            {" "}
            <button type="button" className="dm-sys-act" disabled={context.busy} onClick={action.run}>
              {action.label}
            </button>
          </>
        )}
      </span>
    </div>
  );
}
