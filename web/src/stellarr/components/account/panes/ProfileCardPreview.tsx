import { useRef, type CSSProperties } from "react";

import { Icon } from "@stellarr/icons";
import { Avatar } from "@/components/primitives/Avatar";
import { ProfileBioText } from "@/components/profile/ProfileBio";
import { AvatarUpload } from "@/components/settings/AvatarUpload";
import { defaultIdentityArt, hueFor } from "@/design/identity";
import type { Account, CustomEmoji } from "@/ports";
import { useLanguage } from "@/state/hooks";

interface ProfileCardPreviewProps {
  account: Account;
  /** The bio being edited, which the card shows before it is saved. */
  bio: string;
  /** The same for the pronouns, printed beside the handle as the card does. */
  pronouns?: string;
  customEmoji: readonly CustomEmoji[];
  onPickAvatar: (file: File) => Promise<void> | void;
  onPickBanner: (file: File) => void;
}

/**
 * The card the pane produces, at the size it will be seen, and the control
 * that changes it. Both pictures are real buttons, so the gesture works from
 * the keyboard and each one says what it will do.
 */
export function ProfileCardPreview({
  account,
  bio,
  pronouns = "",
  customEmoji,
  onPickAvatar,
  onPickBanner,
}: ProfileCardPreviewProps) {
  const { t } = useLanguage();
  const bannerInput = useRef<HTMLInputElement>(null);

  const openPicker = (input: HTMLInputElement | null) => {
    if (input === null) return;
    // Cleared before opening as well as after `change`: some browsers keep the
    // old selection when React swaps the image after an upload, which would
    // make choosing the same file again do nothing.
    input.value = "";
    input.click();
  };

  const hasAvatar = typeof account.avatarUrl === "string";
  const hasBanner = typeof account.bannerUrl === "string";

  return (
    <>
      <section className="pf-preview" aria-label={t("account.profileCard")}>
        <button
          type="button"
          className="pf-shot pf-banner"
          style={{ "--mp-hue": hueFor(account.id) } as CSSProperties}
          onClick={() => openPicker(bannerInput.current)}
        >
          <img
            src={account.bannerUrl || defaultIdentityArt(account.id).bannerUrl}
            alt=""
            className={account.bannerUrl ? undefined : "banner-art"}
          />
          <span className="pf-veil">
            <Icon name="camera" size={18} />
            {hasBanner
              ? t("account.changeBackground")
              : t("account.addBackground")}
          </span>
        </button>

        <div className="pf-face">
          <AvatarUpload
            className="pf-shot-av"
            label={hasAvatar ? t("account.changeAvatar") : t("account.addAvatar")}
            shape="circle"
            onSave={onPickAvatar}
          >
            <Avatar
              id={account.id}
              displayName={account.displayName || "?"}
              {...(account.avatarUrl ? { avatarUrl: account.avatarUrl } : {})}
              size="xl"
            />
          </AvatarUpload>

          {/* The name as it will read on the card, so the field below has a
              visible result and does not need to repeat it. */}
          <span className="pf-face-txt">
            <b>{account.displayName}</b>
            <span>
              @{account.username}
              {pronouns.trim().length === 0 ? null : (
                <span className="mp-pronouns">{pronouns.trim()}</span>
              )}
            </span>
          </span>
        </div>

        {bio.trim().length === 0 ? null : (
          <p className="pf-preview-bio">
            <ProfileBioText text={bio} customEmoji={customEmoji} />
          </p>
        )}
      </section>

      <input
        ref={bannerInput}
        type="file"
        accept="image/png,image/jpeg,image/gif,image/webp"
        hidden
        onChange={(event) => {
          const file = event.currentTarget.files?.[0];
          event.currentTarget.value = "";
          if (file !== undefined) onPickBanner(file);
        }}
      />
    </>
  );
}
