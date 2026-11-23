import { Icon } from "@stellarr/icons";
import { Avatar } from "@/components/primitives/Avatar";
import { WallpaperPicture } from "@/components/shell/WallpaperLayer";
import type { WallpaperSettings } from "@/design/themes";
import { useLanguage } from "@/state/hooks";

import styles from "./AppearancePane.module.css";

/**
 * The app in miniature, drawn with the live tokens: a rail, a channel list,
 * and a room whose messages are the transcript's own markup. Every choice in
 * the pane shows here the moment it is made, text size and density included,
 * because the room is laid out by the rules that lay out the real one.
 *
 * A wallpaper is drawn behind it too; the surfaces over it are already
 * see-through, since they read the same tokens the app's do.
 */
export function Stage({ wallpaper }: { wallpaper: { settings: WallpaperSettings; url: string | undefined } | null }) {
  const { t } = useLanguage();
  const shown = wallpaper !== null && wallpaper.url !== undefined;
  return (
    <figure className={shown ? `ap-stage ${styles.stageWithWallpaper}` : "ap-stage"} aria-hidden="true">
      {shown ? (
        <WallpaperPicture
          settings={wallpaper.settings}
          url={wallpaper.url}
          className={`wallpaper ${styles.stageWallpaper}`}
        />
      ) : null}
      <div className="ap-stage-rail">
        <i className="on">ST</i>
        <i>DS</i>
        <i>GM</i>
      </div>
      <div className="ap-stage-side">
        <b>Stellarr</b>
        <span className="ap-stage-ch on">
          <em>#</em>
          {t("theme.stage.general")}
        </span>
        <span className="ap-stage-ch unread">
          <em>#</em>
          {t("theme.stage.design")}
        </span>
        <span className="ap-stage-ch">
          <em>#</em>
          {t("theme.stage.random")}
        </span>
        <span className="ap-stage-ch">
          <Icon name="sound" size={14} />
          {t("theme.stage.lobby")}
        </span>
      </div>
      <div className="ap-stage-room">
        <div className="ap-stage-head">
          <em>#</em>
          {t("theme.stage.general")}
        </div>
        <div className="ap-stage-feed">
          <div className="msg-group">
            <Avatar id="podglad-ada" displayName="Ada" />
            <div className="g-main">
              <p className="msg-head">
                <b>Ada</b>
                <time>19:04</time>
              </p>
              <p className="msg-body">{t("theme.previewMessageOne")}</p>
              <p className="msg-body">{t("theme.previewMessageTwo")}</p>
            </div>
          </div>
          <div className="msg-group">
            <Avatar id="podglad-nils" displayName="Nils" />
            <div className="g-main">
              <p className="msg-head">
                <b>Nils</b>
                <time>19:06</time>
              </p>
              <p className="msg-body">
                <span className="msg-mention">@Ada</span> {t("theme.previewMessageThree")}
              </p>
            </div>
          </div>
        </div>
        <div className="ap-stage-composer">{t("theme.stage.composer")}</div>
      </div>
    </figure>
  );
}
