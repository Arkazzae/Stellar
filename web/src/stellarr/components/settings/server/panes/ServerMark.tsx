import { imageSource } from "@/components/primitives/imageSource";

/** The server's mark: its icon, or its two letters on the accent. */
export function ServerMark({ mark, iconUrl, large = false }: { mark: string; iconUrl?: string; large?: boolean }) {
  const src = imageSource(iconUrl);
  return (
    <span className={large ? "ss-mark ss-mark-lg" : "ss-mark"} aria-hidden="true">
      {src === undefined ? mark : <img src={src} alt="" />}
    </span>
  );
}
