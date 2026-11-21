import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react";

import { personSubject } from "@/components/personSubject";
import { useConfirm } from "@/components/primitives/confirmContext";
import type { DmConversation, EncryptionStatus, EncryptionSwitchAction } from "@/ports";

import { useLanguage } from "./LanguageProvider";
import { usePorts } from "./PortsProvider";
import { useSessionContext } from "./SessionProvider";

/** The one encryption dialog that can be open, and what it is for. */
export type EncryptionDialog =
  /** First PIN; `then` is the switch it was raised for, run once the PIN is set. */
  | { kind: "setup"; then?: { conversationId: string; action: "request" | "confirm" } }
  | { kind: "unlock" }
  | { kind: "forgot" }
  | { kind: "newPin"; mode: "change" | "reset" }
  | { kind: "safety"; conversationId: string };

export interface EncryptionValue {
  status: EncryptionStatus;
  dialog: EncryptionDialog | null;
  openDialog: (dialog: EncryptionDialog) => void;
  closeDialog: () => void;
  /** The conversation whose switch is moving right now. */
  switching: string | null;
  /** The last switch that failed, and why, in words. */
  switchError: { conversationId: string; message: string } | null;
  /** Switch on: asks, or confirms the other person's request; sets the PIN up first when there is none. */
  switchOn: (conversation: DmConversation) => void;
  /** Switch off: withdraws your request, declines theirs, or ends encryption after asking. */
  switchOff: (conversation: DmConversation) => void;
  /** Runs a switch action as it is, from a line or card in the conversation. */
  runSwitch: (conversationId: string, action: EncryptionSwitchAction) => void;
  /** Request lines the reader put aside with "Nie teraz"; local to this tab. */
  dismissedRequests: ReadonlySet<string>;
  dismissRequest: (lineId: string) => void;
}

const EncryptionContext = createContext<EncryptionValue | null>(null);

/**
 * Where encryption stands for the account and which of its dialogs is open.
 *
 * The status is the port's; this adds what the screens share: the switch
 * actions (which may have to set a PIN up first), the dialog any of them can
 * raise, and the requests put aside for now.
 */
export function EncryptionProvider({ children }: { children: ReactNode }) {
  const { encryption } = usePorts();
  const { user } = useSessionContext();
  const { t } = useLanguage();
  const confirm = useConfirm();
  const status = useSyncExternalStore(encryption.subscribe, encryption.getStatus, encryption.getStatus);
  const [dialog, setDialog] = useState<EncryptionDialog | null>(null);
  const [switching, setSwitching] = useState<string | null>(null);
  const [switchError, setSwitchError] = useState<EncryptionValue["switchError"]>(null);
  const [dismissedRequests, setDismissed] = useState<ReadonlySet<string>>(() => new Set());

  const runSwitch = useCallback(
    (conversationId: string, action: EncryptionSwitchAction) => {
      setSwitching(conversationId);
      setSwitchError(null);
      encryption
        .switchConversation(conversationId, action)
        .catch(() => setSwitchError({ conversationId, message: t("e2ee.switch.failed") }))
        .finally(() => setSwitching((current) => (current === conversationId ? null : current)));
    },
    [encryption, t],
  );

  const switchOn = useCallback(
    (conversation: DmConversation) => {
      if (status.support !== "supported") return;
      const state = conversation.encryption;
      const theirs = state?.state === "requested" && state.requestedById !== user?.id;
      if (state?.state === "on" || (state?.state === "requested" && !theirs)) return;
      const action = theirs ? "confirm" : "request";
      if (status.accountKey === null) {
        setDialog({ kind: "setup", then: { conversationId: conversation.id, action } });
        return;
      }
      runSwitch(conversation.id, action);
    },
    [status.support, status.accountKey, user?.id, runSwitch],
  );

  const switchOff = useCallback(
    (conversation: DmConversation) => {
      const state = conversation.encryption;
      if (state === undefined || state.state === "off") return;
      if (state.state === "requested") {
        runSwitch(conversation.id, state.requestedById === user?.id ? "cancel" : "disable");
        return;
      }
      void confirm({
        title: t("e2ee.disable.title"),
        description: t("e2ee.disable.detail"),
        // The conversation is the thing, and the person is how it is known.
        subject: personSubject(conversation.peer),
        confirmLabel: t("e2ee.disable.confirm"),
      }).then((confirmed) => {
        if (confirmed) runSwitch(conversation.id, "disable");
      });
    },
    [user?.id, runSwitch, confirm, t],
  );

  const dismissRequest = useCallback((lineId: string) => {
    setDismissed((current) => new Set([...current, lineId]));
  }, []);

  const closeDialog = useCallback(() => setDialog(null), []);

  const value = useMemo<EncryptionValue>(
    () => ({
      status,
      dialog,
      openDialog: setDialog,
      closeDialog,
      switching,
      switchError,
      switchOn,
      switchOff,
      runSwitch,
      dismissedRequests,
      dismissRequest,
    }),
    [status, dialog, closeDialog, switching, switchError, switchOn, switchOff, runSwitch, dismissedRequests, dismissRequest],
  );

  return <EncryptionContext.Provider value={value}>{children}</EncryptionContext.Provider>;
}

export function useEncryptionContext(): EncryptionValue {
  const value = useContext(EncryptionContext);
  if (value === null) throw new Error("useEncryption must be used inside <EncryptionProvider>");
  return value;
}
