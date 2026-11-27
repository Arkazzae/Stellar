import { useEffect, useId, useRef, useState, type RefObject } from "react";
import { createPortal } from "react-dom";

import { useLayer } from "@stellarr/ui/behavior";
import { Icon } from "@stellarr/icons";
import { trapTab } from "@/components/primitives/focus";
import type { Translator } from "@/i18n";
import { foldCommandText } from "@/lib/messages/composerCommands";

import { CommandIcon } from "./CommandList";
import type { ComposerCommand } from "./commandRegistry";
import css from "./ComposerCommands.module.css";

export interface SheetEntry {
  id: string;
  label: string;
  /** The line beside the label: the command as one would type it, or what the entry covers. */
  detail: string;
  icon: ComposerCommand["icon"] | "textFormat";
  /** Everything the search field finds it by. */
  names: readonly string[];
  disabled?: boolean;
  onSelect(): void;
}

export interface SheetGroup {
  id: string;
  label: string;
  entries: readonly SheetEntry[];
}

export interface CommandSheetProps {
  /** Things to add, drawn as tiles across the top. */
  tiles: readonly SheetEntry[];
  groups: readonly SheetGroup[];
  /** Takes the focus back when the sheet closes, if it is still on the page. */
  returnFocusTo: RefObject<HTMLElement | null>;
  onClose(): void;
  t: Translator;
}

function finds(entry: SheetEntry, query: string): boolean {
  return query === "" || entry.names.some((name) => foldCommandText(name).includes(query));
}

/**
 * The phone's way into the commands, from the plus: a search, the things to
 * add as tiles, then the rest in their groups. It is modal and a layer on the
 * shared stack, so Escape and a press beside it close it, and Tab stays in it.
 * Choosing an entry closes the sheet first, so whatever the entry opens has
 * the screen to itself.
 *
 * It takes the focus itself rather than giving it to the search, which would
 * raise the software keyboard over the entries before anyone asked to type.
 */
export function CommandSheet({ tiles, groups, returnFocusTo, onClose, t }: CommandSheetProps) {
  const [query, setQuery] = useState("");
  const layer = useRef<HTMLDivElement>(null);
  const sheet = useRef<HTMLDivElement>(null);
  const searchId = useId();
  const baseId = useId();

  useLayer(true, {
    node: () => layer.current,
    owns: (target) => {
      const backdrop = layer.current;
      return backdrop !== null && target !== backdrop && backdrop.contains(target);
    },
    onEscape: (event) => {
      event.preventDefault();
      onClose();
    },
    onTab: (event) => {
      if (sheet.current !== null) trapTab(event, sheet.current);
    },
    onPointerDownOutside: () => {
      onClose();
    },
  });

  useEffect(() => {
    const { body } = document;
    const overflow = body.style.overflow;
    const returnTarget = returnFocusTo.current;
    body.style.overflow = "hidden";
    const frame = requestAnimationFrame(() => sheet.current?.focus());
    return () => {
      cancelAnimationFrame(frame);
      body.style.overflow = overflow;
      if (returnTarget?.isConnected) returnTarget.focus();
    };
  }, [returnFocusTo]);

  const needle = foldCommandText(query.trim());
  const shownTiles = tiles.filter((entry) => finds(entry, needle));
  const shownGroups = groups
    .map((group) => ({ ...group, entries: group.entries.filter((entry) => finds(entry, needle)) }))
    .filter((group) => group.entries.length > 0);

  const choose = (entry: SheetEntry) => {
    onClose();
    entry.onSelect();
  };

  const glyph = (icon: SheetEntry["icon"]) =>
    icon === "textFormat" ? <Icon name="textFormat" size={18} /> : <CommandIcon icon={icon} />;

  return createPortal(
    <div ref={layer} className={css.sheetLayer}>
      <div
        ref={sheet}
        className={css.sheet}
        role="dialog"
        aria-modal="true"
        aria-label={t("composer.commands.open")}
        tabIndex={-1}
      >
        <button type="button" className={css.grab} aria-label={t("common.close")} onClick={onClose}>
          <span aria-hidden="true" />
        </button>
        <label className={css.find} htmlFor={searchId}>
          <Icon name="search" size={18} />
          <span className="sr-only">{t("composer.commands.search")}</span>
          <input
            id={searchId}
            type="search"
            value={query}
            placeholder={t("composer.commands.search")}
            autoComplete="off"
            onChange={(event) => setQuery(event.target.value)}
          />
        </label>
        <div className={css.sheetBody}>
          {shownTiles.length === 0 ? null : (
            <div className={css.tiles}>
              {shownTiles.map((entry) => (
                <button
                  key={entry.id}
                  type="button"
                  className={css.tile}
                  disabled={entry.disabled}
                  onClick={() => choose(entry)}
                >
                  <span className={css.tileIcon} aria-hidden="true">
                    {glyph(entry.icon)}
                  </span>
                  {entry.label}
                </button>
              ))}
            </div>
          )}
          {shownGroups.map((group) => {
            const captionId = `${baseId}-${group.id}`;
            return (
              <div key={group.id} role="group" aria-labelledby={captionId}>
                <div id={captionId} className={css.caption}>
                  {group.label}
                </div>
                {group.entries.map((entry) => (
                  <button
                    key={entry.id}
                    type="button"
                    className={css.sheetRow}
                    disabled={entry.disabled}
                    onClick={() => choose(entry)}
                  >
                    <span className={css.icon} aria-hidden="true">
                      {glyph(entry.icon)}
                    </span>{" "}
                    <span className={css.name}>{entry.label}</span>{" "}
                    <span className={css.summary}>{entry.detail}</span>
                  </button>
                ))}
              </div>
            );
          })}
          {shownTiles.length === 0 && shownGroups.length === 0 ? (
            <p className={css.empty}>{t("composer.commands.none")}</p>
          ) : null}
        </div>
      </div>
    </div>,
    document.body,
  );
}
