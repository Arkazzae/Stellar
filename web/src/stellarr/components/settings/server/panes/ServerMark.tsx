/** The server's mark: its icon, or its two letters on the accent. */
export function ServerMark({ mark, iconUrl, large = false }: { mark: string; iconUrl?: string; large?: boolean }) {
  return (
    <span className={large ? "ss-mark ss-mark-lg" : "ss-mark"} aria-hidden="true">
      {iconUrl === undefined ? mark : <img src={iconUrl} alt="" />}
    </span>
  );
}
