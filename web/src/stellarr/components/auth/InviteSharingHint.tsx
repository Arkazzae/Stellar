import type { ReactNode } from "react";
import type { Translator } from "@/i18n";

import controls from "./AuthControls.module.css";

export function InviteSharingHint({ code, t, children }: { code: string | null; t: Translator; children: ReactNode }) {
  if (code === null) return <>{children}</>;
  return (
    <>
      {t("auth.setup.localInvite")}{" "}
      <a className={controls.link} href="https://github.com/Arkazzae/Stellarr#inviting-friends" target="_blank" rel="noreferrer">
        {t("auth.setup.configureAccess")}
      </a>
    </>
  );
}
