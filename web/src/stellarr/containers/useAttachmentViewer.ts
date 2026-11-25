import { useCallback, useMemo, useState } from "react";

import { safeAttachmentUrl, safeHttpUrl } from "@/lib/format/safeUrl";
import type { ViewerTarget } from "@/components/room/attachments/MediaViewer";
import type { AttachmentCallbacks } from "@/lib/messages/parts";

/** Turns an encrypted file's link into a local object URL; null when it cannot be read. */
export type OpenEncrypted = (url: string) => Promise<{ url: string; type: string } | null>;

/** The prefix of a link to an encrypted file that is decrypted only when opened. */
const ENCRYPTED_LINK = "e2ee:";

/**
 * Saves a decrypted file under its own name. A decrypted object URL runs in
 * this page's origin, so it is saved, never opened in a tab.
 */
function save(url: string, name: string): void {
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = name;
  anchor.rel = "noopener";
  document.body.append(anchor);
  anchor.click();
  anchor.remove();
}

/**
 * What the lightbox shows, and the callbacks that let any attachment tile in
 * the room open it. The state is the room's own: nothing else in the app opens
 * the lightbox, so it does not go through the UI provider.
 *
 * `openEncrypted` serves a private conversation's encrypted files: a video is
 * decrypted when it is played and a document when it is downloaded.
 */
export function useAttachmentViewer(openEncrypted?: OpenEncrypted) {
  const [target, setTarget] = useState<ViewerTarget | null>(null);
  const close = useCallback(() => setTarget(null), []);

  const attachments = useMemo<AttachmentCallbacks>(
    () => ({
      onOpenImage: (images, index) => {
        const selected = images[index];
        const safeImages = images.filter((image) => safeAttachmentUrl(image.src) !== undefined);
        const selectedIndex = selected === undefined ? -1 : safeImages.indexOf(selected);
        if (selectedIndex < 0) return;
        const gallery = safeImages.map((image) => ({ ...image, preview: safeAttachmentUrl(image.preview) }));
        setTarget({ kind: "image", images: gallery, index: selectedIndex });
      },
      resolveOriginal: (src) => {
        const url = safeAttachmentUrl(src);
        if (url === undefined || !url.startsWith(ENCRYPTED_LINK)) return Promise.resolve(url ?? null);
        return Promise.resolve(openEncrypted?.(url)).then((opened) => safeAttachmentUrl(opened?.url) ?? null);
      },
      onPlayVideo: (part) => {
        const show = (src: string) =>
          setTarget({
            kind: "video",
            src,
            poster: safeAttachmentUrl(part.thumb),
            ...(part.name === undefined ? {} : { name: part.name }),
          });
        const url = safeAttachmentUrl(part.url);
        if (url === undefined) return;
        if (url.startsWith(ENCRYPTED_LINK)) {
          void openEncrypted?.(url).then((opened) => {
            const src = safeAttachmentUrl(opened?.url);
            if (src !== undefined) show(src);
          });
          return;
        }
        show(url);
      },
      // An embed card and a file are links, and links open where links open;
      // neither is worth a modal.
      onOpenEmbed: (part) => {
        const url = safeHttpUrl(part.url);
        if (url !== undefined) window.open(url, "_blank", "noopener,noreferrer");
      },
      onDownloadFile: (part) => {
        const url = safeAttachmentUrl(part.url);
        if (url === undefined) return;
        if (url.startsWith(ENCRYPTED_LINK)) {
          void openEncrypted?.(url).then((opened) => {
            const src = safeAttachmentUrl(opened?.url);
            if (src?.startsWith("blob:")) save(src, part.name);
          });
          return;
        }
        if (url.startsWith("blob:")) {
          save(url, part.name);
          return;
        }
        window.open(url, "_blank", "noopener,noreferrer");
      },
    }),
    [openEncrypted],
  );

  return { target, close, attachments };
}
