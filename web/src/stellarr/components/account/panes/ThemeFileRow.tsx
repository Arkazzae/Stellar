import { useRef, useState } from "react";

import { Button, Spinner } from "@stellarr/ui";
import { Icon } from "@stellarr/icons";
import { useConfirm } from "@/components/primitives/ConfirmDialog";
import { Switch } from "@/components/primitives/Switch";
import {
  THEME_FILE_MAX_BYTES,
  readThemeFile,
  themeFileName,
  themeFileOf,
  themeFileText,
  type ThemeFileProblem,
} from "@/design/themeFile";
import type { ThemePreference } from "@/design/themes";
import type { MessageKey } from "@/i18n";
import { useLanguage } from "@/state/hooks";

import styles from "./AppearancePane.module.css";
import { customThemeName } from "./PaletteBoard";
import { applyWallpaper, dataAddressOf, type WallpaperAccess } from "./wallpaperUpload";

const PROBLEMS: Record<ThemeFileProblem | "unreadable", MessageKey> = {
  notOurs: "theme.file.notOurs",
  colours: "theme.file.colours",
  tooLarge: "theme.file.tooLarge",
  unreadable: "theme.file.unreadable",
};

/** Hands the browser a file to save, under the given name. */
function download(name: string, text: string) {
  const url = URL.createObjectURL(new Blob([text], { type: "application/json" }));
  const link = document.createElement("a");
  link.href = url;
  link.download = name;
  link.click();
  // The click has started the download; the address is not needed after it.
  setTimeout(() => URL.revokeObjectURL(url), 0);
}

export interface ThemeFileRowProps {
  theme: ThemePreference;
  onChange: (patch: Partial<ThemePreference>) => void;
  /** The account's wallpaper; without it a file's wallpaper is left out. */
  wallpaper?: WallpaperAccess;
}

/**
 * The theme as a file: the palette and accent in use saved to disk,
 * with the wallpaper unless the person leaves it out, and a file picked from
 * disk applied as the person's own palette. Applying one replaces their own
 * palette, so it asks first when there is one; a wallpaper in it is asked
 * about apart, since it goes onto the account.
 */
export function ThemeFileRow({ theme, onChange, wallpaper }: ThemeFileRowProps) {
  const { t } = useLanguage();
  const confirm = useConfirm();
  const input = useRef<HTMLInputElement>(null);
  const [message, setMessage] = useState<{ text: string; error: boolean } | null>(null);
  const [busy, setBusy] = useState(false);
  const [withWallpaper, setWithWallpaper] = useState(true);
  const hasWallpaper = theme.wallpaper !== null && wallpaper?.url !== undefined;

  const save = async () => {
    setMessage(null);
    let image: string | undefined;
    if (hasWallpaper && withWallpaper && wallpaper !== undefined) {
      setBusy(true);
      try {
        image = await dataAddressOf(await wallpaper.read());
      } catch {
        setMessage({ text: t("theme.file.wallpaperUnreadable"), error: true });
      } finally {
        setBusy(false);
      }
    }
    const file = themeFileOf(theme, image);
    download(themeFileName(file.name, t("theme.custom.untitled")), themeFileText(file));
  };

  const load = async (file: File) => {
    setMessage(null);
    if (file.size > THEME_FILE_MAX_BYTES) {
      setMessage({ text: t(PROBLEMS.tooLarge), error: true });
      return;
    }
    let text: string;
    try {
      text = await file.text();
    } catch {
      setMessage({ text: t(PROBLEMS.unreadable), error: true });
      return;
    }
    const read = readThemeFile(text);
    if (!read.ok) {
      setMessage({ text: t(PROBLEMS[read.problem]), error: true });
      return;
    }
    if (
      theme.custom !== null &&
      !(await confirm({
        title: t("theme.custom.replaceTitle"),
        description: t("theme.file.replaceDetail"),
        subject: { kind: "item", name: customThemeName(theme.custom, t("theme.custom.untitled")), mark: { icon: "palette" } },
        confirmLabel: t("theme.custom.replaceConfirm"),
      }))
    ) {
      return;
    }
    onChange(read.patch);

    if (read.wallpaper === undefined || wallpaper === undefined) return;
    const yes = await confirm({
      title: t("theme.file.wallpaperTitle"),
      description: t("theme.file.wallpaperDetail"),
      confirmLabel: t("theme.file.wallpaperConfirm"),
      cancelLabel: t("theme.file.wallpaperSkip"),
    });
    if (!yes) return;
    setBusy(true);
    // The rest of the theme stays applied whatever happens to the picture.
    const outcome = await applyWallpaper(read.wallpaper.image, wallpaper, theme, onChange, t, read.wallpaper);
    setBusy(false);
    if (!outcome.ok) setMessage({ text: outcome.message, error: true });
  };

  return (
    <>
      <div className="frm-row ap-row">
        <span className="frm-lab">
          {t("theme.file.title")}
          <span
            className={message?.error === true ? "frm-detail-error" : undefined}
            role={message === null ? undefined : "alert"}
          >
            {busy ? t("theme.wallpaper.uploading") : (message?.text ?? t("theme.file.detail"))}
          </span>
        </span>
        <div className={`frm-ctl ap-end ${styles.actions}`}>
          <Button type="button" variant="secondary" size="sm" disabled={busy} onClick={() => void save()}>
            {busy ? <Spinner /> : <Icon name="download" size={15} />}
            {t("theme.file.save")}
          </Button>
          <Button type="button" variant="secondary" size="sm" disabled={busy} onClick={() => input.current?.click()}>
            <Icon name="upload" size={15} />
            {t("theme.file.load")}
          </Button>
          <input
            ref={input}
            type="file"
            className="sr-only"
            tabIndex={-1}
            aria-hidden="true"
            accept=".json,application/json"
            onChange={(event) => {
              const picked = event.currentTarget.files?.[0];
              // Cleared, so the same file can be picked again after a change.
              event.currentTarget.value = "";
              if (picked !== undefined) void load(picked);
            }}
          />
        </div>
      </div>
      {hasWallpaper ? (
        <div className="frm-row">
          <span className="frm-lab">
            {t("theme.file.withWallpaper")}
            <span>{t("theme.file.withWallpaperDetail")}</span>
          </span>
          <div className="frm-ctl frm-switch">
            <Switch
              checked={withWallpaper}
              onChange={setWithWallpaper}
              label={t("theme.file.withWallpaper")}
            />
          </div>
        </div>
      ) : null}
    </>
  );
}
