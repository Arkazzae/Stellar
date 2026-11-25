import { useId, useRef, useState, type CSSProperties } from "react";

import { Button, Spinner, cx } from "@stellarr/ui";
import { Icon } from "@stellarr/icons";
import card from "@/components/primitives/ConfirmDialog.module.css";
import { useConfirm } from "@/components/primitives/ConfirmDialog";
import { Modal } from "@/components/primitives/Modal";
import { Switch } from "@/components/primitives/Switch";
import { makeCustomTheme } from "@/design/customTheme";
import { FONT_LABELS } from "@/design/fontLabels";
import {
  CUSTOM_NAME_MAX,
  CUSTOM_PALETTE,
  WALLPAPER_SHADE_DEFAULT,
  normaliseCustomTheme,
  normaliseText,
  paletteCssVariables,
  type ThemePreference,
} from "@/design/themes";
import type { MessageKey } from "@/i18n";
import type { Conversion } from "@/lib/betterDiscord/convert";
import type { SourceGroup } from "@/lib/betterDiscord/sources";
import { useLanguage } from "@/state/hooks";

import styles from "./BetterDiscordDialog.module.css";
import { customThemeName } from "./PaletteBoard";
import { Stage } from "./ThemeStage";
import { applyWallpaper, type WallpaperAccess } from "./wallpaperUpload";

/** The converter, fetched the first time the dialog is asked to read a theme. */
const converter = () => import("@/lib/betterDiscord/run");

/** What a theme's stylesheets may hold together; `INLINE_LIMITS.total` repeated, so the check needs no converter. */
const THEME_MAX_CHARS = 2 * 1024 * 1024;

const ADDRESS_PROBLEMS: Record<"notHttps" | "notFound" | "invalid", MessageKey> = {
  notHttps: "theme.bd.notHttps",
  notFound: "theme.bd.notFound",
  invalid: "theme.bd.invalidAddress",
};

type State =
  | { step: "idle" }
  | { step: "loading" }
  | { step: "failed"; message: string }
  | { step: "results"; groups: SourceGroup[] }
  | { step: "ready"; conversion: Conversion; source?: string };

export interface BetterDiscordDialogProps {
  open: boolean;
  onClose: () => void;
  theme: ThemePreference;
  onChange: (patch: Partial<ThemePreference>) => void;
  wallpaper?: WallpaperAccess;
}

/** Resolve a source, preview its conversion and apply the choices the user confirms. */
export default function BetterDiscordDialog({ open, onClose, theme, onChange, wallpaper }: BetterDiscordDialogProps) {
  const { t } = useLanguage();
  const confirm = useConfirm();
  const titleId = useId();
  const addressId = useId();
  const input = useRef<HTMLInputElement>(null);
  const [address, setAddress] = useState("");
  const [state, setState] = useState<State>({ step: "idle" });
  const [fetchPicture, setFetchPicture] = useState(false);
  const [applying, setApplying] = useState(false);
  const [applyError, setApplyError] = useState<string | null>(null);

  const read = async (convert: () => Promise<State>) => {
    setState({ step: "loading" });
    setApplyError(null);
    setFetchPicture(false);
    try {
      setState(await convert());
    } catch {
      setState({ step: "failed", message: t("theme.bd.failed") });
    }
  };

  const readFile = (file: File) =>
    void read(async () => {
      const { convertText } = await converter();
      return { step: "ready", conversion: await convertText(await file.text(), file.name) };
    });

  const readAddress = (value: string) =>
    void read(async () => {
      const { convertAddress } = await converter();
      const converted = await convertAddress(value);
      if ("problem" in converted) return { step: "failed", message: t(ADDRESS_PROBLEMS[converted.problem]) };
      if ("groups" in converted) return { step: "results", groups: converted.groups };
      return { step: "ready", conversion: converted.conversion, source: converted.source };
    });

  const apply = async (conversion: Conversion, source: string | undefined) => {
    const font = conversion.font?.id ?? undefined;
    const name = normaliseText(conversion.meta.name ?? "", CUSTOM_NAME_MAX);
    const patch: Partial<ThemePreference> = {
      // A palette brings its own colour as the accent when the theme set none.
      ...((conversion.accent ?? conversion.palette?.accent) === undefined
        ? {}
        : { accent: conversion.accent ?? conversion.palette?.accent }),
      ...(font === undefined ? {} : { font }),
    };
    if (conversion.palette !== undefined) {
      const custom = normaliseCustomTheme(
        makeCustomTheme({
          name,
          ...(conversion.meta.author === undefined ? {} : { author: conversion.meta.author }),
          ...((conversion.meta.source ?? source) === undefined ? {} : { source: conversion.meta.source ?? source }),
          colors: conversion.palette,
        }),
      );
      if (custom === null) return;
      if (
        theme.custom !== null &&
        !(await confirm({
          title: t("theme.custom.replaceTitle"),
          description: t("theme.bd.replaceDetail"),
          subject: { kind: "item", name: customThemeName(theme.custom, t("theme.custom.untitled")), mark: { icon: "palette" } },
          confirmLabel: t("theme.custom.replaceConfirm"),
        }))
      ) {
        return;
      }
      Object.assign(patch, { palette: CUSTOM_PALETTE, custom, scheme: "dark" });
    }
    if (fetchPicture && conversion.wallpaper !== undefined && wallpaper !== undefined) {
      setApplying(true);
      let picture: Blob;
      try {
        picture = await (await converter()).fetchPicture(conversion.wallpaper.url);
      } catch {
        setApplying(false);
        setApplyError(t("theme.bd.pictureUnreachable"));
        return;
      }
      const outcome = await applyWallpaper(picture, wallpaper, theme, (wallpaperPatch) => {
        Object.assign(patch, wallpaperPatch);
      }, t, {
        shade: conversion.wallpaper.shade ?? WALLPAPER_SHADE_DEFAULT,
        blur: 0,
      });
      setApplying(false);
      if (!outcome.ok) {
        setApplyError(outcome.message);
        return;
      }
    }
    if (Object.keys(patch).length > 0) onChange(patch);
    onClose();
  };

  const isBusy = state.step === "loading" || applying;

  return (
    <Modal open={open} onClose={applying ? () => undefined : onClose} labelledBy={titleId} className={cx(card.wrap)} exitMs={200}>
      <div className={cx(card.card, styles.card)} aria-busy={isBusy || undefined}>
        <div className={card.body}>
          <h2 id={titleId}>{t("theme.bd.title")}</h2>
          <p className={card.sub}>{t("theme.bd.intro")}</p>
        </div>

        <div className={styles.source}>
          <Button type="button" variant="secondary" size="sm" disabled={isBusy} onClick={() => input.current?.click()}>
            <Icon name="upload" size={15} />
            {t("theme.bd.pickFile")}
          </Button>
          <input
            ref={input}
            type="file"
            className="sr-only"
            tabIndex={-1}
            aria-hidden="true"
            accept=".css,text/css"
            onChange={(event) => {
              const file = event.currentTarget.files?.[0];
              event.currentTarget.value = "";
              if (file === undefined) return;
              if (file.size > THEME_MAX_CHARS) {
                setState({ step: "failed", message: t("theme.bd.tooLarge") });
                return;
              }
              readFile(file);
            }}
          />
          <form
            className={styles.address}
            onSubmit={(event) => {
              event.preventDefault();
              readAddress(address.trim() || "https://betterdiscord.app/themes");
            }}
          >
            <label className="sr-only" htmlFor={addressId}>
              {t("theme.bd.address")}
            </label>
            <input
              id={addressId}
              className="set-input"
              type="text"
              maxLength={4096}
              placeholder={t("theme.bd.addressPlaceholder")}
              value={address}
              spellCheck={false}
              autoComplete="off"
              onChange={(event) => setAddress(event.currentTarget.value)}
            />
            <Button type="submit" variant="secondary" size="sm" disabled={isBusy}>
              {t(address.trim() ? "theme.bd.read" : "theme.bd.browse")}
            </Button>
          </form>
        </div>

        {state.step === "loading" ? (
          <p className={styles.status} role="status">
            <Spinner /> {t("theme.bd.reading")}
          </p>
        ) : null}
        {state.step === "failed" ? (
          <p className={styles.error} role="alert">
            {state.message}
          </p>
        ) : null}
        {state.step === "results" ? (
          <div className={styles.result} aria-live="polite">
            {state.groups.map(({ provider, choices, problem }) => (
              <section key={provider} className={styles.searchGroup}>
                <h3>{t(`theme.bd.provider.${provider}`)}</h3>
                {problem !== undefined ? (
                  <p>{t(problem === "rateLimited" ? "theme.bd.rateLimited" : "theme.bd.searchUnavailable")}</p>
                ) : choices.length === 0 ? (
                  <p>{t(provider === "files" ? "theme.bd.noFiles" : "theme.bd.noResults")}</p>
                ) : (
                  <ul>
                    {choices.map(({ label, detail, url }) => (
                      <li key={url}>
                        <button type="button" onClick={() => readAddress(url)}>
                          <span>{label}</span>
                          {detail === undefined ? null : <small>{detail}</small>}
                        </button>
                      </li>
                    ))}
                  </ul>
                )}
              </section>
            ))}
          </div>
        ) : null}
        {state.step === "ready" ? (
          <Result
            conversion={state.conversion}
            fetchPicture={fetchPicture}
            onFetchPicture={setFetchPicture}
            canSetWallpaper={wallpaper !== undefined}
          />
        ) : null}
        {applyError === null ? null : (
          <p className={styles.error} role="alert">
            {applyError}
          </p>
        )}

        <div className={card.foot}>
          <Button type="button" variant="secondary" disabled={applying} onClick={onClose}>
            {t("common.cancel")}
          </Button>
          {state.step === "ready" && (state.conversion.palette !== undefined || state.conversion.accent !== undefined || state.conversion.font?.id || (fetchPicture && state.conversion.wallpaper !== undefined)) ? (
            <Button
              type="button"
              variant="primary"
              disabled={applying}
              onClick={() => void apply(state.conversion, state.source)}
            >
              {applying ? <Spinner /> : null}
              {state.conversion.palette === undefined ? t("theme.bd.applyPartial") : t("theme.bd.apply")}
            </Button>
          ) : null}
        </div>
      </div>
    </Modal>
  );
}

/** What came across, in the miniature and in words, and what did not. */
function Result({
  conversion,
  fetchPicture,
  onFetchPicture,
  canSetWallpaper,
}: {
  conversion: Conversion;
  fetchPicture: boolean;
  onFetchPicture: (value: boolean) => void;
  canSetWallpaper: boolean;
}) {
  const { t } = useLanguage();
  const { meta, palette, font, wallpaper } = conversion;
  const filled = conversion.roles.filter((role) => role.from === undefined && role.group !== "accent");
  const preview =
    palette === undefined
      ? null
      : (paletteCssVariables(makeCustomTheme({ name: "", colors: palette })) as CSSProperties);

  const taken: string[] = [];
  if (palette !== undefined) {
    const shadedBy = conversion.roles.find((role) => role.group === "backgrounds" && role.role === "primary")?.from;
    taken.push(
      conversion.backgroundsFromShading !== true
        ? t("theme.bd.takenPalette")
        : shadedBy === undefined
          ? t("theme.bd.takenUnshadedPalette")
          : t("theme.bd.takenShadedPalette"),
    );
  }
  if (conversion.accent !== undefined) taken.push(t("theme.bd.takenAccent"));
  if (font?.id != null) taken.push(t("theme.bd.takenFont", { font: FONT_LABELS[font.id].label }));
  if (wallpaper !== undefined && canSetWallpaper) taken.push(t("theme.bd.takenWallpaper"));

  const left: string[] = [t("theme.bd.leftLayout")];
  if (palette === undefined) left.push(t("theme.bd.leftPalette"));
  if (font !== undefined && font.id === null) left.push(t("theme.bd.leftFont", { font: font.family }));
  if (filled.length > 0) left.push(t("theme.bd.leftFilled"));
  if (conversion.skipped.length > 0) left.push(t("theme.bd.leftImports", { count: conversion.skipped.length }));

  return (
    <div className={styles.result}>
      {preview === null ? null : (
        <div
          className={styles.preview}
          data-theme="custom"
          data-scheme="dark"
          {...(font?.id == null ? {} : { "data-font": font.id })}
          style={preview}
        >
          <Stage wallpaper={null} />
        </div>
      )}
      <p className={styles.meta}>
        <b>{meta.name ?? t("theme.custom.untitled")}</b>
        {meta.author === undefined ? null : <span>{t("theme.bd.by", { author: meta.author })}</span>}
        {meta.version === undefined ? null : <span>{meta.version}</span>}
      </p>
      <div className={styles.lists}>
        <section aria-label={t("theme.bd.taken")}>
          <h3>{t("theme.bd.taken")}</h3>
          {taken.length === 0 ? <p>{t("theme.bd.nothingTaken")}</p> : <ul>{taken.map((line) => <li key={line}>{line}</li>)}</ul>}
        </section>
        <section aria-label={t("theme.bd.left")}>
          <h3>{t("theme.bd.left")}</h3>
          <ul>{left.map((line) => <li key={line}>{line}</li>)}</ul>
        </section>
      </div>
      {wallpaper === undefined || !canSetWallpaper ? null : (
        <div className={styles.picture}>
          <span>
            {t("theme.bd.picture", { host: wallpaper.host })}
            <small>{t("theme.bd.pictureDetail")}</small>
          </span>
          <Switch checked={fetchPicture} onChange={onFetchPicture} label={t("theme.bd.picture", { host: wallpaper.host })} />
        </div>
      )}
    </div>
  );
}
