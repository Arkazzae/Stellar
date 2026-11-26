import { useId, useState } from "react";

import { Button, cx } from "@stellarr/ui";
import { Icon } from "@stellarr/icons";
import { ConfirmError } from "@/components/primitives/ConfirmDialog";
import card from "@/components/primitives/ConfirmDialog.module.css";
import { Modal } from "@/components/primitives/Modal";
import { parseThemeCss, themeCssSource } from "@/design/themeCss";
import { CUSTOM_PALETTE, type ThemePreference } from "@/design/themes";
import { useLanguage } from "@/state/hooks";

import styles from "./AppearancePane.module.css";
import { customThemeName } from "./PaletteBoard";

export function ThemeCssRow({ theme, onChange }: {
  theme: ThemePreference;
  onChange: (patch: Partial<ThemePreference>) => void;
}) {
  const { t } = useLanguage();
  const titleId = useId();
  const inputId = useId();
  const [draft, setDraft] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const close = () => setDraft(null);

  const save = () => {
    if (draft === null) return;
    const untitled = t("theme.custom.untitled");
    const name = theme.custom === null ? untitled : customThemeName(theme.custom, untitled);
    const result = parseThemeCss(draft, theme, name);
    if ("problem" in result) {
      setError(t(`theme.css.${result.problem}`, { property: result.property ?? "" }));
      return;
    }
    onChange({ palette: CUSTOM_PALETTE, accent: result.custom.colors.accent, custom: result.custom });
    close();
  };

  return (
    <div className="frm-row ap-row">
      <span className="frm-lab">
        {t("theme.css.title")}
        <span>{t("theme.css.detail")}</span>
      </span>
      <div className="frm-ctl ap-end">
        <Button type="button" variant="secondary" size="sm" onClick={() => {
          setError(null);
          setDraft(themeCssSource(theme));
        }}>
          <Icon name="code" size={15} />
          {t("theme.css.edit")}
        </Button>
      </div>
      <Modal open={draft !== null} onClose={close} labelledBy={titleId} className={card.wrap ?? ""}>
        <div className={cx(card.card, styles.cssCard)}>
          <div className={cx(card.body, styles.cssBody)}>
            <h2 id={titleId}>{t("theme.css.title")}</h2>
            <p className={card.sub}>{t("theme.css.help")}</p>
            <div className={card.field}>
              <label className={card.label} htmlFor={inputId}>{t("theme.css.source")}</label>
              <textarea
                id={inputId}
                className={cx("set-area", styles.cssEditor)}
                spellCheck={false}
                autoCapitalize="off"
                aria-invalid={error === null ? undefined : true}
                value={draft ?? ""}
                onChange={(event) => {
                  setDraft(event.target.value);
                  setError(null);
                }}
              />
              <a className={styles.cssDocs} href="https://arkazzae.github.io/Stellar/themes.html#write-a-css-theme" target="_blank" rel="noreferrer">
                {t("theme.css.docs")}
              </a>
            </div>
            {error === null ? null : <ConfirmError text={error} />}
          </div>
          <div className={card.foot}>
            <Button type="button" variant="secondary" onClick={close}>{t("common.cancel")}</Button>
            <Button type="button" variant="primary" onClick={save}>{t("common.save")}</Button>
          </div>
        </div>
      </Modal>
    </div>
  );
}
