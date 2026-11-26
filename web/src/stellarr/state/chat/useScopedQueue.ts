import { useCallback, useEffect, useRef, useState } from "react";
import { composerDraft, readDraftFiles, writeDraftFiles } from "@/lib/platform/drafts";
import type { PendingChip } from "./pendingChips";

const EMPTY: PendingChip[] = [];

interface Queue {
  chips: PendingChip[];
  restored: boolean;
}

/** A late upload or send still changes the queue of its original destination. */
export function useScopedQueue(scope: string | undefined) {
  const [queues, setQueues] = useState(() => new Map<string, Queue>());
  const latest = useRef(queues);
  const publish = useCallback((id: string, queue: Queue) => {
    const next = new Map(latest.current);
    next.set(id, queue);
    latest.current = next;
    setQueues(next);
  }, []);

  const change = useCallback((update: (items: PendingChip[]) => PendingChip[]) => {
    if (!scope) return;
    const queue = latest.current.get(scope);
    const before = queue?.chips ?? EMPTY;
    const after = update(before);
    publish(scope, { chips: after, restored: queue?.restored ?? false });
    if (before.length !== after.length || before.some((item, index) => item.id !== after[index]?.id)) {
      void writeDraftFiles(scope, before, after).catch(() => composerDraft(scope).storageFailed());
    }
  }, [publish, scope]);

  useEffect(() => {
    if (!scope || latest.current.get(scope)?.restored) return;
    let active = true;
    void readDraftFiles(scope).catch(() => []).then(files => {
      if (!active) return;
      const current = latest.current.get(scope)?.chips ?? EMPTY;
      const restored = files.filter(file => !current.some(chip => chip.id === file.id))
        .map((item): PendingChip => ({ ...item, name: item.file.name, status: "restored", attachment: null }));
      publish(scope, { chips: restored.length ? [...restored, ...current] : current, restored: true });
    });
    return () => { active = false; };
  }, [publish, scope]);

  const queue = scope ? queues.get(scope) : undefined;
  return [queue?.chips ?? EMPTY, change, !!scope && !queue?.restored] as const;
}
