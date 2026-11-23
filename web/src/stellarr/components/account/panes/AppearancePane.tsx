import { useId, type CSSProperties } from "react";

import { Button } from "@stellarr/ui";
import { Icon, type IconName } from "@stellarr/icons";
import { Segmented, type SegmentedOption } from "@/components/primitives/Segmented";
import { Switch } from "@/components/primitives/Switch";
import { useRovingRadio } from "@/components/primitives/useRovingRadio";
import { ACCENT_SWATCHES } from "@/design/accentSwatches";
import { FONT_LABELS } from "@/design/fontLabels";
import {
  CUSTOM_PALETTE,
  DEFAULT_THEME,
  DENSITIES,
  FONTS,
  PALETTES,
  SIZES,
  paletteCssVariables,
  resolvePalette,
  type DensityPreference,
  type FontId,
  type PaletteChoice,
  type ResolvedPalette,
  type ResolvedScheme,
  type SchemePreference,
  type SizePreference,
  type ThemePreference,
} from "@/design/themes";
import { useLanguage } from "@/state/hooks";

import styles from "./AppearancePane.module.css";
import { BetterDiscordRow } from "./BetterDiscordRow";
import { PaletteBoard, customThemeName } from "./PaletteBoard";
import { Stage } from "./ThemeStage";
import { ThemeFileRow } from "./ThemeFileRow";
import { WallpaperSection } from "./WallpaperSection";
import type { WallpaperAccess } from "./wallpaperUpload";

/**
 * Reduced motion. It is an account preference with its own owner rather than
 * an axis of the theme, but it answers the same question as contrast and text
 * size, how the app reads, so it is set here.
 */
export interface MotionPreferenceProps {
  reducedMotion: boolean;
  onChange: (reducedMotion: boolean) => void;
}

export interface AppearancePaneProps {
  theme: ThemePreference;
  /** Which of dark and light is actually in effect; the preset tiles follow it. */
  resolvedScheme: ResolvedScheme;
  onChange: (patch: Partial<ThemePreference>) => void;
  /** Every axis back to `DEFAULT_THEME`, in one write. */
  onReset: () => void;
  motion?: MotionPreferenceProps;
  /** The account's wallpaper; without it the pane offers none. */
  wallpaper?: WallpaperAccess;
}

const SCHEME_ICONS: Record<SchemePreference, IconName> = {
  system: "monitor",
  dark: "moon",
  light: "sun",
};

/** The value of the swatch row for the theme's own colour. */
const THEME_ACCENT = "theme";

interface Swatch {
  value: string;
  hex: string;
  label: string;
  /** The theme's own colour, set apart from the curated ones by a rule. */
  theme?: boolean;
}

/**
 * A row of colour swatches as one radio group. A swatch is the colour itself,
 * so the name is its accessible name and its tooltip.
 */
function SwatchGroup({
  label,
  swatches,
  value,
  onChange,
}: {
  label: string;
  swatches: readonly Swatch[];
  value: string;
  onChange: (value: string) => void;
}) {
  const roving = useRovingRadio(
    swatches.length,
    swatches.findIndex((swatch) => swatch.value === value),
    (index) => onChange(swatches[index]!.value),
  );

  return (
    <div className="ap-swatches" role="radiogroup" aria-label={label}>
      {swatches.map((swatch, index) => {
        const on = swatch.value === value;
        return (
          <button
            key={swatch.value}
            ref={roving.ref(index)}
            type="button"
            role="radio"
            aria-checked={on}
            aria-label={swatch.label}
            title={swatch.label}
            tabIndex={roving.tabIndex(index)}
            className={["ap-acc", swatch.theme ? "theme" : "", on ? "on" : ""].filter(Boolean).join(" ")}
            style={{ "--sw": swatch.hex } as CSSProperties}
            onClick={() => onChange(swatch.value)}
            onKeyDown={(event) => roving.onKeyDown(event, index)}
          />
        );
      })}
    </div>
  );
}

const FONT_KINDS = ["sans", "rounded", "mono"] as const;

/**
 * The face, as a list of names rather than a sheet of specimens: a name set
 * in its own face would download every face just by opening the pane. The
 * miniature above shows the chosen one, since it is set by the same rule as
 * the app.
 */
function FontRow({ font, onChange }: { font: FontId; onChange: (font: FontId) => void }) {
  const { t } = useLanguage();
  const id = useId();
  return (
    <div className="frm-row ap-row">
      <label className="frm-lab" htmlFor={id}>
        {t("theme.font")}
        <span>{t("theme.fontDetail")}</span>
      </label>
      <div className="frm-ctl ap-end">
        <select
          id={id}
          className="set-select"
          value={font}
          onChange={(event) => onChange(event.currentTarget.value as FontId)}
        >
          {FONT_KINDS.map((kind) => (
            <optgroup key={kind} label={t(`theme.fontKind.${kind}`)}>
              {FONTS.filter((face) => FONT_LABELS[face].kind === kind).map((face) => (
                <option key={face} value={face}>
                  {FONT_LABELS[face].label}
                </option>
              ))}
            </optgroup>
          ))}
        </select>
      </div>
    </div>
  );
}

/**
 * What colour the app is, and what face and size its conversation is set in.
 *
 * The pane opens on the app in miniature, then the choices, one line each.
 * Nothing here names a colour: a preset tile carries its own CSS variables and
 * `data-scheme`, so an entry added to the registry paints its own swatch.
 */
export function AppearancePane({
  theme,
  resolvedScheme,
  onChange,
  onReset,
  motion,
  wallpaper,
}: AppearancePaneProps) {
  const { t } = useLanguage();

  const preset = resolvePalette(theme);
  const accent = theme.accent.toLowerCase();
  const presetAccent = preset.colors.accent.toLowerCase();
  // The theme's own colour leads the row, and a curated colour the preset
  // already uses is not offered twice.
  const swatches = ACCENT_SWATCHES.filter((swatch) => swatch.hex.toLowerCase() !== presetAccent);
  const accentValue =
    accent === presetAccent
      ? THEME_ACCENT
      : (swatches.find((swatch) => swatch.hex.toLowerCase() === accent)?.id ?? "");
  const customAccent = accentValue === "";

  const accentSwatches: readonly Swatch[] = [
    { value: THEME_ACCENT, hex: preset.colors.accent, label: t("theme.themeColor"), theme: true },
    ...swatches.map((swatch) => ({
      value: swatch.id,
      hex: swatch.hex,
      label: t(`theme.accent.${swatch.id}`),
    })),
  ];

  const schemeOptions: readonly SegmentedOption<SchemePreference>[] = (
    ["system", "dark", "light"] as const
  ).map((value) => ({
    value,
    label: t(`theme.scheme.${value}`),
    title: t(`theme.scheme.${value}Detail`),
    leading: <Icon name={SCHEME_ICONS[value]} size={16} />,
  }));
  const sizeOptions: readonly SegmentedOption<SizePreference>[] = SIZES.map(({ id }) => ({
    value: id,
    label: t(`theme.size.${id}`),
    title: t(`theme.size.${id}Detail`),
    // The sample is set by the same token as the room, at the size it offers.
    leading: (
      <span className="ap-aa" data-size={id} aria-hidden="true">
        Aa
      </span>
    ),
  }));
  const densityOptions: readonly SegmentedOption<DensityPreference>[] = DENSITIES.map(({ id }) => ({
    value: id,
    label: t(`theme.density.${id}`),
    title: t(`theme.density.${id}Detail`),
  }));

  // The custom palette is kept through a reset, so it does not count here.
  const isDefault =
    (Object.keys(DEFAULT_THEME) as (keyof ThemePreference)[]).every(
      (axis) => axis === "custom" || axis === "wallpaper" || theme[axis] === DEFAULT_THEME[axis],
    ) && motion?.reducedMotion !== true;

  // "Restore defaults" means everything this pane shows, motion included.
  const resetAll = () => {
    onReset();
    if (motion?.reducedMotion) motion.onChange(false);
  };

  // The person's own palette sits among the presets once there is one, and
  // paints its tile from its own variables the way they do.
  const tiles: readonly { id: PaletteChoice; label: string; title: string; palette: ResolvedPalette }[] = [
    ...PALETTES.map((palette) => ({
      id: palette.id,
      label: palette.label,
      title: t(`theme.palette.${palette.id}`),
      palette,
    })),
    ...(theme.custom === null
      ? []
      : [
          {
            id: CUSTOM_PALETTE as PaletteChoice,
            label: customThemeName(theme.custom, t("theme.custom.untitled")),
            title: theme.custom.author ?? t("theme.custom.untitled"),
            palette: theme.custom,
          },
        ]),
  ];

  const pickAccent = (value: string) => {
    if (value === THEME_ACCENT) {
      onChange({ accent: preset.colors.accent });
      return;
    }
    const swatch = swatches.find((item) => item.id === value);
    if (swatch) onChange({ accent: swatch.hex });
  };

  return (
    <>
      <Stage wallpaper={theme.wallpaper === null ? null : { settings: theme.wallpaper, url: wallpaper?.url }} />

      <section className="sc" aria-label={t("theme.theme")}>
        <header className="sc-h">{t("theme.theme")}</header>
        <div className="sc-b ap-theme">
          <div
            className={theme.custom === null ? "ap-themes" : `ap-themes ${styles.withOwn}`}
            role="radiogroup"
            aria-label={t("theme.theme")}
          >
            {tiles.map((tile) => {
              const selected = tile.id === theme.palette;
              return (
                <button
                  key={tile.id}
                  type="button"
                  role="radio"
                  aria-checked={selected}
                  title={tile.title}
                  className={selected ? "ap-tile on" : "ap-tile"}
                  // The tile is a working instance of the theme it offers:
                  // these attributes are the contract `applyThemeAttributes`
                  // themes the document by, so everything inside resolves to
                  // its own tokens.
                  data-theme={tile.id}
                  data-scheme={resolvedScheme}
                  data-contrast={theme.contrast}
                  style={paletteCssVariables(tile.palette) as CSSProperties}
                  onClick={() => onChange({ palette: tile.id, accent: tile.palette.colors.accent })}
                >
                  <span className="ap-sw" aria-hidden="true">
                    <i />
                    <i />
                    <i />
                    <b />
                  </span>
                  <span className="ap-name">{tile.label}</span>
                  {selected ? (
                    <span className="ap-tick" aria-hidden="true">
                      <Icon name="tick" size={12} />
                    </span>
                  ) : null}
                </button>
              );
            })}
          </div>

          <div className="frm">
            <div className="frm-row ap-row">
              <span className="frm-lab">
                {t("theme.brightness")}
                <span>{t("theme.systemBrightnessDetail")}</span>
              </span>
              <div className="frm-ctl ap-end">
                <Segmented
                  options={schemeOptions}
                  value={theme.scheme}
                  onChange={(scheme) => onChange({ scheme })}
                  label={t("theme.brightness")}
                  variant="pill"
                  className="ap-seg"
                />
              </div>
            </div>

            <div className="frm-row ap-row">
              <span className="frm-lab">
                {t("theme.accentColor")}
                <span>{t("theme.accentDetail")}</span>
              </span>
              <div className="frm-ctl ap-end ap-accents">
                <SwatchGroup
                  label={t("theme.accentColor")}
                  swatches={accentSwatches}
                  value={accentValue}
                  onChange={pickAccent}
                />
                {/* Any other colour: the system picker, behind a hue wheel
                    that fills with the colour once it is the one in use. */}
                <label
                  className={customAccent ? "ap-acc custom on" : "ap-acc custom"}
                  style={{ "--sw": theme.accent } as CSSProperties}
                  title={t("theme.customAccent")}
                >
                  <input
                    type="color"
                    className="sr-only"
                    value={theme.accent}
                    aria-label={t("theme.customAccent")}
                    onChange={(event) => onChange({ accent: event.currentTarget.value })}
                  />
                </label>
              </div>
            </div>

            <div className="frm-row">
              <span className="frm-lab">
                {t("theme.highContrast")}
                <span>{t("theme.contrast.highDetail")}</span>
              </span>
              <div className="frm-ctl frm-switch">
                <Switch
                  checked={theme.contrast === "high"}
                  onChange={(high) => onChange({ contrast: high ? "high" : "normal" })}
                  label={t("theme.highContrast")}
                />
              </div>
            </div>

            <ThemeFileRow theme={theme} onChange={onChange} {...(wallpaper === undefined ? {} : { wallpaper })} />
            <BetterDiscordRow theme={theme} onChange={onChange} {...(wallpaper === undefined ? {} : { wallpaper })} />
          </div>

          <PaletteBoard theme={theme} onChange={onChange} />
        </div>
      </section>

      {wallpaper === undefined ? null : <WallpaperSection theme={theme} onChange={onChange} wallpaper={wallpaper} />}

      <section className="sc" aria-label={t("theme.readability")}>
        <header className="sc-h">{t("theme.readability")}</header>
        <div className="sc-b">
          <div className="frm">
            <FontRow font={theme.font} onChange={(font) => onChange({ font })} />

            <div className="frm-row ap-row">
              <span className="frm-lab">
                {t("theme.textSize")}
                <span>{t("theme.textSizeDetail")}</span>
              </span>
              <div className="frm-ctl ap-end">
                <Segmented
                  options={sizeOptions}
                  value={theme.size}
                  onChange={(size) => onChange({ size })}
                  label={t("theme.textSize")}
                  variant="pill"
                  className="ap-seg"
                />
              </div>
            </div>

            <div className="frm-row ap-row">
              <span className="frm-lab">
                {t("theme.density")}
                <span>{t("theme.densityDetail")}</span>
              </span>
              <div className="frm-ctl ap-end">
                <Segmented
                  options={densityOptions}
                  value={theme.density}
                  onChange={(density) => onChange({ density })}
                  label={t("theme.density")}
                  variant="pill"
                  className="ap-seg"
                />
              </div>
            </div>

            {motion === undefined ? null : (
              <div className="frm-row">
                <span className="frm-lab">
                  {t("preference.reducedMotion")}
                  <span>
                    {t("preference.reducedMotionDetail")} {t("preference.reducedMotionNote")}
                  </span>
                </span>
                <div className="frm-ctl frm-switch">
                  <Switch
                    checked={motion.reducedMotion}
                    onChange={motion.onChange}
                    label={t("preference.reducedMotion")}
                  />
                </div>
              </div>
            )}
          </div>
        </div>
      </section>

      <div className="ap-foot">
        <p>{t("theme.appearanceNote")}</p>
        <Button
          type="button"
          variant="quiet"
          size="sm"
          disabled={isDefault}
          onClick={resetAll}
        >
          <Icon name="refresh" size={15} />
          {t("common.resetDefaults")}
        </Button>
      </div>
    </>
  );
}
