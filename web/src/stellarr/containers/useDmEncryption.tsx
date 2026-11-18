import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";

import { Button } from "@stellarr/ui";
import { Icon } from "@stellarr/icons";
import type { EncryptionSwitchView } from "@/components/friends/DmEncryptionRows";
import { DmNotice, type DmNoticeContext } from "@/components/friends/DmNotice";
import { unsupportedReason } from "@/components/friends/encryptionCopy";
import type { MessageGroupVM, TranscriptNotice } from "@/components/room/transcript/MessageGroup";
import { ENCRYPTION_OFF } from "@/lib/api/types";
import { formatDate } from "@/lib/format/format";
import type { DmConversation } from "@/ports";
import { useEncryption, useFriends, useLanguage, usePorts, useSession } from "@/state/hooks";

/** Where the switch stands as this reader sees it. */
export function switchView(conversation: DmConversation, selfId: string | undefined): EncryptionSwitchView {
  const encryption = conversation.encryption ?? ENCRYPTION_OFF;
  if (encryption.state === "on") return "on";
  if (encryption.state === "requested") return encryption.requestedById === selfId ? "waiting" : "asked";
  return "off";
}

/**
 * What the room of one conversation shows about encryption: the lock in the
 * header, the intro's sentence and button, the lines and sealed blocks in the
 * history, and a composer that gives way when this browser cannot encrypt.
 */
export function useDmEncryptionView(conversation: DmConversation | null) {
  const encryption = useEncryption();
  const friends = useFriends();
  const { user } = useSession();
  const { t, locale } = useLanguage();
  const { status, switchOn, runSwitch, openDialog, dismissRequest, dismissedRequests, switching } = encryption;

  const state = conversation?.encryption ?? ENCRYPTION_OFF;
  const reason = unsupportedReason(t, status.support);
  const on = state.state === "on";
  const conversationId = conversation?.id ?? null;
  const peerName = conversation?.peer.displayName ?? "";
  const { liveRequestId, declinedIds } = friends.encryptionLines;

  const context = useMemo<DmNoticeContext | null>(() => {
    if (conversation === null) return null;
    return {
      selfId: user?.id ?? "",
      peerName,
      liveRequestId,
      declinedIds,
      dismissedRequests,
      encryptionOn: on,
      canEnterPin:
        status.support === "supported" && status.accountKey !== null && status.backup !== null && !status.unlocked,
      unsupportedReason: reason,
      busy: switching === conversation.id,
      onCancelRequest: () => runSwitch(conversation.id, "cancel"),
      onConfirmRequest: () => switchOn(conversation),
      onDismissRequest: dismissRequest,
      onCompareCode: () => openDialog({ kind: "safety", conversationId: conversation.id }),
      onEnterPin: () => openDialog({ kind: "unlock" }),
    };
  }, [conversation, user?.id, peerName, liveRequestId, declinedIds, dismissedRequests, on, status, reason, switching, runSwitch, switchOn, dismissRequest, openDialog]);

  const renderNotice = useCallback(
    (notice: TranscriptNotice, group: MessageGroupVM): ReactNode =>
      context === null ? null : (
        <DmNotice notice={notice} lineId={group.id} authorId={group.authorId} context={context} />
      ),
    [context],
  );

  if (conversation === null || conversationId === null) return {};

  const composerBlocked = !on
    ? undefined
    : status.support === "insecure"
      ? t("e2ee.composer.insecure")
      : status.support === "unsupported"
        ? t("e2ee.composer.unsupported")
        : status.deviceLimit
          ? t("e2ee.composer.deviceLimit")
          : undefined;

  return {
    renderNotice,
    ...(on && state.enabledAt !== null
      ? { introText: t("e2ee.intro.encrypted", { date: formatDate(state.enabledAt, locale) }) }
      : {}),
    ...(state.state === "off" && reason === null
      ? {
          introAction: (
            <Button type="button" variant="secondary" size="sm" onClick={() => switchOn(conversation)}>
              <Icon name="lock" size={15} />
              {t("e2ee.intro.enable")}
            </Button>
          ),
        }
      : {}),
    ...(composerBlocked === undefined ? {} : { composerBlocked }),
  };
}

export type SafetyStanding = { status: "unverified" | "changed" } | { status: "verified"; on: string };

/**
 * Whether the person compared this conversation's safety code, and whether it
 * is still the code they compared. Looked up again whenever keys change.
 */
export function useSafetyStanding(conversationId: string | null, active: boolean): SafetyStanding | undefined {
  const { encryption: port } = usePorts();
  const { status } = useEncryption();
  const { locale } = useLanguage();
  const [standing, setStanding] = useState<SafetyStanding | undefined>(undefined);
  const { revision, support } = status;

  useEffect(() => {
    if (!active || conversationId === null || support !== "supported") return;
    let current = true;
    void (async () => {
      const code = await port.safetyCode(conversationId);
      if (code === null) return;
      const verified = await port.verification(code.peerId);
      const joined = code.groups.join(" ");
      if (!current) return;
      setStanding(
        verified === null
          ? { status: "unverified" }
          : verified.code === joined
            ? { status: "verified", on: formatDate(verified.at, locale) }
            : { status: "changed" },
      );
    })().catch(() => undefined);
    return () => {
      current = false;
    };
  }, [port, conversationId, active, support, revision, locale]);

  return active ? standing : undefined;
}
