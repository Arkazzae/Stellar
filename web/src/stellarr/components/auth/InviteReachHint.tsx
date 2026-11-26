import type { ReactNode } from "react";
import type { Translator } from "@/i18n";

import controls from "./AuthControls.module.css";

/** Check the generated link, which can use a different host from the setup page. */
function isLoopbackInvite(url: string): boolean {
  try {
    const host = new URL(url).hostname.replace(/\.$/, "");
    return host === "localhost" || host.endsWith(".localhost") || /^127(?:\.\d+){3}$/.test(host) ||
      host === "[::1]" || /^\[::ffff:7f[\da-f]{2}:/.test(host);
  } catch {
    return false;
  }
}

export function InviteReachHint({ url, t, children }: { url: string; t: Translator; children: ReactNode }) {
  if (!isLoopbackInvite(url)) return <>{children}</>;
  return (
    <>
      {t("auth.setup.localInvite")}{" "}
      <a className={controls.link} href="https://github.com/Arkazzae/Stellarr#inviting-friends" target="_blank" rel="noreferrer">
        {t("auth.setup.configureAccess")}
      </a>
    </>
  );
}
