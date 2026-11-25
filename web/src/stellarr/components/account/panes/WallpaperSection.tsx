import { useId, useRef, useState } from "react";

import { Button, Spinner } from "@stellarr/ui";
import { Icon } from "@stellarr/icons";
import { useConfirm } from "@/components/primitives/ConfirmDialog";
import { WALLPAPER_BLUR_MAX, WALLPAPER_SHADE_DEFAULT, type ThemePreference } from "@/design/themes";
import { useLanguage } from "@/state/hooks";

import styles from "./AppearancePane.module.css";
import { RangeField } from "./VoiceFields";
import { applyWallpaper, type WallpaperAccess } from "./wallpaperUpload";

/** What the picker offers; anything else is still decoded if the browser can. */
const ACCEPT = "image/jpeg,image/png,image/webp,image/avif,image/gif";

export interface WallpaperSectionProps {
  theme: ThemePreference;
  onChange: (patch: Partial<ThemePreference>) => void;
  wallpaper: WallpaperAccess;
}

/**
 * A picture behind the whole app: choosing one, how much the theme's
 * background shades it, how much it is blurred, and taking it away. The
 * picture is the account's, like the avatar, but nobody else sees it.
 */
export function WallpaperSection({ theme, onChange, wallpaper }: WallpaperSectionProps) {
  const { t } = useLanguage();
  const confirm = useConfirm();
  const input = useRef<HTMLInputElement>(null);
  const shadeId = useId();
  const blurId = useId();
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<{ text: string; error: boolean } | null>(null);
  const hasPicture = wallpaper.url !== undefined;
  // The shading and blur belong to a picture there is.
  const settings = hasPicture ? theme.wallpaper : null;

  const pick = async (file: File) => {
    setBusy(true);
    setMessage(null);
    const outcome = await applyWallpaper(file, wallpaper, theme, onChange, t);
    setBusy(false);
    if (!outcome.ok) setMessage({ text: outcome.message, error: true });
    else if (outcome.firstFrameOnly) setMessage({ text: t("theme.wallpaper.firstFrame"), error: false });
  };

  const remove = async () => {
    const yes = await confirm({
      title: t("theme.wallpaper.removeTitle"),
      description: t("theme.wallpaper.removeDetail"),
      confirmLabel: t("theme.wallpaper.remove"),
      danger: true,
    });
    if (!yes) return;
    setMessage(null);
    try {
      await wallpaper.remove();
      onChange({ wallpaper: null });
    } catch {
      setMessage({ text: t("theme.wallpaper.removeFailed"), error: true });
    }
  };

  const update = (patch: { shade?: number; blur?: number }) => {
    if (settings !== null) onChange({ wallpaper: { ...settings, ...patch } });
  };

  return (
    <section className="sc" aria-label={t("theme.wallpaper.title")}>
      <header className="sc-h">{t("theme.wallpaper.title")}</header>
      <div className="sc-b">
        <div className="frm">
          <div className="frm-row ap-row">
            <span className="frm-lab">
              {t("theme.wallpaper.picture")}
              <span
                className={message?.error === true ? "frm-detail-error" : undefined}
                role={message === null ? undefined : message.error ? "alert" : "status"}
              >
                {busy ? t("theme.wallpaper.uploading") : (message?.text ?? t("theme.wallpaper.detail"))}
              </span>
            </span>
            <div className={`frm-ctl ap-end ${styles.actions}`}>
              {hasPicture && settings === null ? (
                <Button
                  type="button"
                  variant="secondary"
                  size="sm"
                  disabled={busy}
                  onClick={() => onChange({ wallpaper: { shade: WALLPAPER_SHADE_DEFAULT, blur: 0 } })}
                >
                  {t("theme.wallpaper.show")}
                </Button>
              ) : null}
              <Button type="button" variant="secondary" size="sm" disabled={busy} onClick={() => input.current?.click()}>
                {busy ? <Spinner /> : <Icon name="image" size={15} />}
                {hasPicture ? t("theme.wallpaper.change") : t("theme.wallpaper.choose")}
              </Button>
              {hasPicture ? (
                <Button type="button" variant="quiet" size="sm" disabled={busy} onClick={() => void remove()}>
                  <Icon name="trash" size={15} />
                  {t("theme.wallpaper.remove")}
                </Button>
              ) : null}
              <input
                ref={input}
                type="file"
                className="sr-only"
                tabIndex={-1}
                aria-hidden="true"
                accept={ACCEPT}
                onChange={(event) => {
                  const file = event.currentTarget.files?.[0];
                  event.currentTarget.value = "";
                  if (file !== undefined) void pick(file);
                }}
              />
            </div>
          </div>

          {settings === null ? null : (
            <>
              <div className="frm-row ap-row">
                <label className="frm-lab" htmlFor={shadeId}>
                  {t("theme.wallpaper.shade")}
                  <span>{t("theme.wallpaper.shadeDetail")}</span>
                </label>
                <div className="frm-ctl ap-end">
                  <RangeField
                    id={shadeId}
                    min={0}
                    max={100}
                    step={5}
                    value={settings.shade}
                    display={`${settings.shade}%`}
                    label={t("theme.wallpaper.shade")}
                    valueText={`${settings.shade}%`}
                    onChange={(shade) => update({ shade })}
                  />
                </div>
              </div>
              <div className="frm-row ap-row">
                <label className="frm-lab" htmlFor={blurId}>
                  {t("theme.wallpaper.blur")}
                  <span>{t("theme.wallpaper.blurDetail")}</span>
                </label>
                <div className="frm-ctl ap-end">
                  <RangeField
                    id={blurId}
                    min={0}
                    max={WALLPAPER_BLUR_MAX}
                    step={1}
                    value={settings.blur}
                    display={t("theme.wallpaper.blurValue", { value: settings.blur })}
                    label={t("theme.wallpaper.blur")}
                    valueText={t("theme.wallpaper.blurValue", { value: settings.blur })}
                    onChange={(blur) => update({ blur })}
                  />
                </div>
              </div>
            </>
          )}
        </div>
      </div>
    </section>
  );
}
