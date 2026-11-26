import { useCallback, useMemo } from "react";

import { useScopedQueue } from "./useScopedQueue";
import { describeError } from "@/lib/api/errors";
import type { Translator } from "@/i18n";
import type { Attachment } from "@/ports";

import {
  dropChips,
  markFailed,
  markUploaded,
  markUploading,
  moveChip,
  queueFiles,
  toComposerChips,
} from "./pendingChips";

/**
 * The files waiting to go out with the next message.
 *
 * Each uploads as soon as it is added, and the composer keeps send disabled
 * until every chip has a real server id, so a message can never outrun its
 * media upload.
 */
export function useAttachmentQueue(
  uploadFile: (file: File) => Promise<Attachment>, t: Translator, scope: string | undefined, dm = false,
) {
  const [pending, setPending, restoringFiles] = useScopedQueue(scope);

  const upload = useCallback(
    (id: string, file: File) => {
      setPending((current) => markUploading(current, id));
      void uploadFile(file)
        .then((attachment) => {
          setPending((current) => markUploaded(current, id, attachment));
        })
        .catch((error: unknown) => {
          const reason = dm ? t("attachment.uploadFailed") : describeError(error, t("composer.attachmentUploadFailed"), t);
          setPending((current) => markFailed(current, id, reason));
        });
    },
    [uploadFile, t, setPending, dm],
  );

  const attach = useCallback(
    (files: File[]) => {
      if (!scope) return;
      const queued = queueFiles(files, pending.length, () => (dm ? "dm-chip-" : "chip-") + crypto.randomUUID());
      setPending((current) => [...current, ...queued]);
      for (const chip of queued) upload(chip.id, chip.file);
    },
    [pending.length, upload, setPending, dm, scope],
  );

  const retryChip = useCallback(
    (chipId: string) => {
      const failed = pending.find(
        (chip) => chip.id === chipId && (chip.status === "error" || chip.status === "restored"),
      );
      if (failed !== undefined) upload(failed.id, failed.file);
    },
    [pending, upload],
  );

  /** Forgets chips whose message went out. */
  const discard = useCallback(
    (ids: ReadonlySet<string>) => setPending((current) => dropChips(current, ids)),
    [setPending],
  );

  const removeChip = useCallback(
    (chipId: string) => discard(new Set([chipId])),
    [discard],
  );

  const reorderChips = useCallback(
    (from: number, to: number) =>
      setPending((current) => moveChip(current, from, to)),
    [setPending],
  );

  const reupload = useCallback(() => { for (const chip of pending) upload(chip.id, chip.file); }, [pending, upload]);
  const chips = useMemo(() => toComposerChips(pending), [pending]);

  return useMemo(
    () => ({ pending, chips, restoringFiles, attach, retryChip, removeChip, reorderChips, discard, reupload }),
    [pending, chips, restoringFiles, attach, retryChip, removeChip, reorderChips, discard, reupload],
  );
}
