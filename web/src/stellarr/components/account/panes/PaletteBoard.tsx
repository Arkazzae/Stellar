import { useRef, useState, type CSSProperties, type KeyboardEvent } from "react";

import { Button } from "@stellarr/ui";
import { Icon } from "@stellarr/icons";
import { useConfirm } from "@/components/primitives/ConfirmDialog";
import { forkPreset, withColour } from "@/design/customTheme";
import {
  COLOUR_ROLES,
  CUSTOM_NAME_MAX,
  CUSTOM_PALETTE,
  DEFAULT_THEME,
  getPalette,
  normaliseColour,
  normaliseText,
  resolvePalette,
  type ColourGroup,
  type CustomTheme,
  type PaletteId,
  type ThemePreference,
} from "@/design/themes";
import type { MessageKey } from "@/i18n";
import { useLanguage } from "@/state/hooks";

import styles from "./PaletteBoard.module.css";

/** The groups in the order a palette is built up: surfaces, what is read on them, the lines between, and what carries meaning. */
const GROUPS: readonly { group: ColourGroup; title: MessageKey }[] = [
  { group: "backgrounds", title: "theme.paletteBackgrounds" },
  { group: "text", title: "theme.paletteText" },
  { group: "borders", title: "theme.paletteBorders" },
  { group: "semantic", title: "theme.paletteStates" },
];

/** The custom palette's name as the pane shows it. */
export function customThemeName(custom: CustomTheme, untitled: string): string {
  return custom.name === "" ? untitled : custom.name;
}

/** A typed code read as a colour: `#` may be left out; anything but 6 or 8 hex digits is not one. */
function readTypedColour(text: string): string | null {
  const trimmed = text.trim();
  return normaliseColour(trimmed.startsWith("#") ? trimmed : `#${trimmed}`);
}

/**
 * One role: the colour itself, which opens the system picker, its name, and
 * its code, which takes `#rrggbbaa` too. The picker has no opacity, so a pick
 * keeps the opacity the role already had and the code is where it changes.
 */
function Swatch({
  name,
  value,
  onPick,
}: {
  name: string;
  value: string;
  onPick: (value: string) => void;
}) {
  const { t } = useLanguage();
  const [draft, setDraft] = useState(value);
  const [shown, setShown] = useState(value);
  if (shown !== value) {
    setShown(value);
    setDraft(value);
  }
  const valid = readTypedColour(draft) !== null;

  const commit = () => {
    const colour = readTypedColour(draft);
    if (colour === null) {
      setDraft(value);
      return;
    }
    setDraft(colour);
    if (colour !== value) onPick(colour);
  };
  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter") {
      // Committing may raise the question card, whose first button takes the
      // focus; the same Enter must not go on to press it.
      event.preventDefault();
      commit();
    }
    if (event.key === "Escape" && draft !== value) {
      // The pane's own Escape closes the settings; here it only undoes the typing.
      event.stopPropagation();
      setDraft(value);
    }
  };

  return (
    <li className={styles.swatch}>
      <label className={styles.chip} style={{ "--sw": value } as CSSProperties} title={t("theme.colorPick", { name })}>
        <input
          type="color"
          className="sr-only"
          value={value.slice(0, 7)}
          aria-label={t("theme.colorPick", { name })}
          onChange={(event) => onPick(`${event.currentTarget.value.toLowerCase()}${value.slice(7)}`)}
        />
      </label>
      <span className={styles.name}>{name}</span>
      <input
        className={styles.code}
        value={draft}
        maxLength={9}
        spellCheck={false}
        autoComplete="off"
        aria-label={t("theme.colorHex", { name })}
        aria-invalid={!valid || undefined}
        title={t("theme.colorHexHint")}
        onChange={(event) => setDraft(event.currentTarget.value)}
        onBlur={commit}
        onKeyDown={onKeyDown}
      />
    </li>
  );
}

/** The custom palette's name, saved when the field is left so a space typed between words survives. */
function NameField({ custom, onRename }: { custom: CustomTheme; onRename: (name: string) => void }) {
  const { t } = useLanguage();
  const [draft, setDraft] = useState(custom.name);
  const [shown, setShown] = useState(custom.name);
  if (shown !== custom.name) {
    setShown(custom.name);
    setDraft(custom.name);
  }
  const commit = () => {
    const name = normaliseText(draft, CUSTOM_NAME_MAX);
    setDraft(name);
    if (name !== custom.name) onRename(name);
  };
  return (
    <label className={styles.nameRow}>
      <span>{t("theme.custom.name")}</span>
      <input
        className="set-input"
        value={draft}
        maxLength={CUSTOM_NAME_MAX}
        placeholder={t("theme.custom.untitled")}
        onChange={(event) => setDraft(event.currentTarget.value)}
        onBlur={commit}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            commit();
          }
        }}
      />
    </label>
  );
}

export interface PaletteBoardProps {
  theme: ThemePreference;
  onChange: (patch: Partial<ThemePreference>) => void;
}

/**
 * The full palette: every colour of the palette in use, grouped, each one
 * editable. A change to a preset makes a copy of it the person's own palette
 * and selects it; a change to their own palette changes it. There is one
 * custom palette, so a copy that would replace another one asks first.
 */
export function PaletteBoard({ theme, onChange }: PaletteBoardProps) {
  const { t } = useLanguage();
  const confirm = useConfirm();
  const palette = resolvePalette(theme);
  const own = theme.palette === CUSTOM_PALETTE ? theme.custom : null;
  // While the question is open the picker keeps reporting; the last pick wins.
  const asking = useRef<{ group: ColourGroup; role: string; value: string } | null>(null);

  const edit = (group: ColourGroup, role: string, value: string) => {
    if (own !== null) {
      onChange({ custom: withColour(own, group, role, value) });
      return;
    }
    const preset = theme.palette as PaletteId;
    const fork = () => {
      const pick = asking.current ?? { group, role, value };
      const named = forkPreset(preset, t("theme.custom.forkName", { name: getPalette(preset).label }));
      onChange({ palette: CUSTOM_PALETTE, custom: withColour(named, pick.group, pick.role, pick.value) });
    };
    if (theme.custom === null) {
      fork();
      return;
    }
    const waiting = asking.current !== null;
    asking.current = { group, role, value };
    if (waiting) return;
    void confirm({
      title: t("theme.custom.replaceTitle"),
      description: t("theme.custom.replaceDetail"),
      subject: { kind: "item", name: customThemeName(theme.custom, t("theme.custom.untitled")), mark: { icon: "palette" } },
      confirmLabel: t("theme.custom.replaceConfirm"),
    }).then((yes) => {
      if (yes) fork();
      asking.current = null;
    });
  };

  const discard = async (custom: CustomTheme) => {
    const yes = await confirm({
      title: t("theme.custom.discardTitle"),
      description: t("theme.custom.discardDetail"),
      subject: { kind: "item", name: customThemeName(custom, t("theme.custom.untitled")), mark: { icon: "palette" } },
      confirmLabel: t("theme.custom.discardConfirm"),
      danger: true,
    });
    if (!yes) return;
    if (theme.palette !== CUSTOM_PALETTE) {
      onChange({ custom: null });
      return;
    }
    const back = getPalette(custom.basedOn ?? (DEFAULT_THEME.palette as PaletteId));
    onChange({
      palette: back.id,
      accent: back.colors.accent,
      custom: null,
      ...(back.id === DEFAULT_THEME.palette ? { wallpaper: null } : {}),
    });
  };

  const label = own !== null ? customThemeName(own, t("theme.custom.untitled")) : getPalette(theme.palette as PaletteId).label;

  return (
    <details className={styles.board}>
      <summary>
        {t("theme.fullPalette")} <span>{label}</span>
      </summary>
      {own === null ? null : (
        <NameField custom={own} onRename={(name) => onChange({ custom: { ...own, name } })} />
      )}
      <div className={styles.groups}>
        {GROUPS.map(({ group, title }) => (
          <section key={group} className={styles.group} aria-label={t(title)}>
            <h3>{t(title)}</h3>
            <ul>
              {COLOUR_ROLES[group].map((role) => (
                <Swatch
                  key={role}
                  name={t(`theme.color.${role}`)}
                  value={(palette.colors[group] as Record<string, string>)[role] ?? ""}
                  onPick={(value) => edit(group, role, value)}
                />
              ))}
            </ul>
          </section>
        ))}
      </div>
      <p className={styles.note}>{own === null ? t("theme.paletteForkNote") : t("theme.paletteNote")}</p>
      {theme.custom === null ? null : (
        <div className={styles.actions}>
          <Button type="button" variant="quiet" size="sm" onClick={() => theme.custom && void discard(theme.custom)}>
            <Icon name="trash" size={15} />
            {t("theme.custom.discard")}
          </Button>
        </div>
      )}
    </details>
  );
}
