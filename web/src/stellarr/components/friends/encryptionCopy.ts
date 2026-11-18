import type { MessageKey, Translator } from "@/i18n";
import type { EncryptionStatus, TimelineLine } from "@/ports";

/**
 * The words for one encryption line, shared by the transcript and the rail's
 * preview so the two never describe the same entry differently. Present-tense
 * and colon forms keep Polish free of the actor's grammatical gender.
 */
export function lineText(
  t: Translator,
  line: TimelineLine,
  actorName: string,
  options: { self: boolean; declined?: boolean; live?: boolean; peerName?: string },
): string {
  const { self } = options;
  switch (line.kind) {
    case "encryption_requested":
      if (self) {
        return options.live
          ? t("e2ee.line.requestedSelf", { name: options.peerName ?? "" })
          : t("e2ee.line.requestedSelfPast");
      }
      return options.live
        ? t("e2ee.line.requestedPeer", { name: actorName })
        : t("e2ee.line.requestedPeerPast", { name: actorName });
    case "encryption_request_cancelled":
      return self ? t("e2ee.line.cancelledSelf") : t("e2ee.line.cancelledPeer", { name: actorName });
    case "encryption_enabled":
      return t("e2ee.line.enabled");
    case "encryption_disabled":
      if (options.declined) {
        return self ? t("e2ee.line.declinedSelf") : t("e2ee.line.declinedPeer", { name: actorName });
      }
      return self ? t("e2ee.line.disabledSelf") : t("e2ee.line.disabledPeer", { name: actorName });
    case "device_added":
      return self ? t("e2ee.line.deviceSelf") : t("e2ee.line.devicePeer", { name: actorName });
    case "keys_reset":
      return self ? t("e2ee.line.resetSelf") : t("e2ee.line.resetPeer", { name: actorName });
  }
}

/** Why this browser cannot take part, in words; null when it can (or is still finding out). */
export function unsupportedReason(t: Translator, support: EncryptionStatus["support"]): string | null {
  if (support === "insecure") return t("e2ee.switch.insecure");
  if (support === "unsupported") return t("e2ee.switch.unsupported");
  return null;
}

const PLURAL: Record<"one" | "few" | "many", MessageKey> = {
  one: "e2ee.sealed.one",
  few: "e2ee.sealed.few",
  many: "e2ee.sealed.many",
};

export function sealedCountKey(form: "one" | "few" | "many"): MessageKey {
  return PLURAL[form];
}
