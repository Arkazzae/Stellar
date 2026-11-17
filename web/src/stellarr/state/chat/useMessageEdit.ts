import { useCallback, useMemo, useRef, useState } from "react";

import type { Message, MessageActionsPort } from "@/ports";

import type { ChatDispatch } from "./shared";

/**
 * The message being rewritten, and the text it started with.
 *
 * The composer is the editor: a message becomes editable in the one field that
 * is already there, already grows, already takes Enter to send. A second
 * textarea drawn inside the transcript would be a second implementation of
 * every one of those.
 */
export function useMessageEdit(
  actions: MessageActionsPort,
  messages: Readonly<Record<string, readonly Message[]>>,
  dispatch: ChatDispatch,
) {
  const [editing, setEditing] = useState<{ id: string; original: string } | null>(
    null,
  );

  const pending = useRef(new Map<string, { cancelled: boolean }>());

  /**
   * Saves a rewrite. The transcript is updated straight away and rolled back
   * if the server refuses, the same bargain sending makes. A failure leaves
   * the editor open with the text still in it, because the alternative is
   * losing what somebody just wrote.
   */
  const saveEdit = useCallback(
    async (messageId: string, text: string) => {
      const next = text.trim();
      if (next.length === 0) return;

      const before = Object.values(messages)
        .flat()
        .find((message) => message.id === messageId);
      if (before === undefined) return;
      if (next === before.content) {
        setEditing(null);
        return;
      }

      const operation = { cancelled: false };
      pending.current.set(messageId, operation);
      const editedAt = new Date().toISOString();
      dispatch({
        type: "rewritten",
        channelId: before.channelId,
        messageId,
        content: next,
        editedAt,
      });
      setEditing(null);

      try {
        await actions.edit(messageId, next);
      } catch (error) {
        dispatch({ type: "edit-rollback", before, content: next, editedAt });
        if (!operation.cancelled && pending.current.get(messageId) === operation) {
          setEditing({ id: messageId, original: before.content });
        }
        throw error;
      } finally {
        if (pending.current.get(messageId) === operation) pending.current.delete(messageId);
      }
    },
    [actions, messages, dispatch],
  );

  const beginEdit = useCallback(
    (message: Message) =>
      setEditing({ id: message.id, original: message.content }),
    [],
  );

  const cancelEdit = useCallback(() => setEditing(null), []);

  /** A message was deleted: stop editing it. */
  const forgetEdit = useCallback(
    (messageId: string) => {
      const operation = pending.current.get(messageId);
      if (operation) operation.cancelled = true;
      setEditing((current) => (current?.id === messageId ? null : current));
    },
    [],
  );

  return useMemo(
    () => ({ editing, saveEdit, beginEdit, cancelEdit, forgetEdit }),
    [editing, saveEdit, beginEdit, cancelEdit, forgetEdit],
  );
}
