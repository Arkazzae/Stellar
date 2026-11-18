import type { ConversationEncryption, Message, SealedReason } from "@/ports";

/**
 * Consecutive messages this browser cannot read become one entry that counts
 * them, so a new browser without the PIN shows one block instead of a wall of
 * placeholders. Runs are split where the reason changes, since "enter the PIN"
 * and "cannot be read here" ask different things of the reader.
 *
 * `cache` keeps the entry for an unchanged run the same object between calls,
 * which is what the transcript presenter keys its work on.
 */
export function collapseSealed(
  messages: readonly Message[],
  cache: Map<string, Message>,
): Message[] {
  const out: Message[] = [];
  const used = new Set<string>();
  let run: Message[] = [];
  let reason: SealedReason | null = null;

  const flush = () => {
    const first = run[0];
    if (first !== undefined && reason !== null) {
      if (run.length === 1) {
        out.push(first);
      } else {
        const key = `${first.id}:${run.length}:${reason}`;
        let entry = cache.get(key);
        if (entry === undefined) {
          entry = { ...first, sealed: { reason, count: run.length } };
          cache.set(key, entry);
        }
        used.add(key);
        out.push(entry);
      }
    }
    run = [];
    reason = null;
  };

  for (const message of messages) {
    const sealed = message.sealed;
    if (sealed === undefined) {
      flush();
      out.push(message);
      continue;
    }
    if (reason !== null && reason !== sealed.reason) flush();
    reason = sealed.reason;
    run.push(message);
  }
  flush();

  for (const key of cache.keys()) if (!used.has(key)) cache.delete(key);
  return out;
}

/** A `collapseSealed` with a cache of its own, for one mounted transcript. */
export function createSealedCollapser(): (messages: readonly Message[]) => Message[] {
  const cache = new Map<string, Message>();
  return (messages) => collapseSealed(messages, cache);
}

export interface EncryptionLineFacts {
  /** The request line still waiting for an answer, if the switch is still "requested". */
  liveRequestId: string | null;
  /** Switch-off lines that answered a request rather than ended encryption. */
  declinedIds: ReadonlySet<string>;
}

/** What the switch lines in a history mean, read in order against where the switch stands. */
export function encryptionLineFacts(
  messages: readonly Message[],
  encryption: ConversationEncryption | undefined,
): EncryptionLineFacts {
  const declinedIds = new Set<string>();
  let lastRequest: Message | null = null;
  let requestOpen = false;
  for (const message of messages) {
    const kind = message.line?.kind;
    if (kind === "encryption_requested") {
      lastRequest = message;
      requestOpen = true;
    } else if (kind === "encryption_disabled") {
      if (requestOpen) declinedIds.add(message.id);
      requestOpen = false;
    } else if (kind === "encryption_enabled" || kind === "encryption_request_cancelled") {
      requestOpen = false;
    }
  }
  const live =
    encryption?.state === "requested" &&
    lastRequest !== null &&
    requestOpen &&
    lastRequest.line?.actorId === encryption.requestedById;
  return { liveRequestId: live && lastRequest !== null ? lastRequest.id : null, declinedIds };
}
