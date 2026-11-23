import { useState } from "react";

import { Button } from "@stellarr/ui";
import { Icon } from "@stellarr/icons";
import type { ThemePreference } from "@/design/themes";
import { useLanguage } from "@/state/hooks";

import BetterDiscordDialog from "./BetterDiscordDialog";
import type { WallpaperAccess } from "./wallpaperUpload";

export interface BetterDiscordRowProps {
  theme: ThemePreference;
  onChange: (patch: Partial<ThemePreference>) => void;
  wallpaper?: WallpaperAccess;
}

/**
 * The way into bringing a BetterDiscord theme over. The dialog is small and
 * comes with the pane; the converter it runs loads when it first reads a
 * theme.
 */
export function BetterDiscordRow({ theme, onChange, wallpaper }: BetterDiscordRowProps) {
  const { t } = useLanguage();
  // Kept mounted once opened, so closing can play out.
  const [opened, setOpened] = useState(false);
  const [open, setOpen] = useState(false);

  return (
    <div className="frm-row ap-row">
      <span className="frm-lab">
        {t("theme.bd.row")}
        <span>{t("theme.bd.rowDetail")}</span>
      </span>
      <div className="frm-ctl ap-end">
        <Button
          type="button"
          variant="secondary"
          size="sm"
          onClick={() => {
            setOpened(true);
            setOpen(true);
          }}
        >
          <Icon name="sparkle" size={15} />
          {t("theme.bd.open")}
        </Button>
      </div>
      {opened ? (
        <BetterDiscordDialog
          open={open}
          onClose={() => setOpen(false)}
          theme={theme}
          onChange={onChange}
          {...(wallpaper === undefined ? {} : { wallpaper })}
        />
      ) : null}
    </div>
  );
}
