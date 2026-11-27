import { useLayoutEffect, useRef, type ReactNode, type Ref } from "react";

import { Icon } from "@stellarr/icons";
import type { Translator } from "@/i18n";

import { COMMAND_GROUPS, type ComposerCommand } from "./commandRegistry";
import css from "./ComposerCommands.module.css";
import type { CommandItem } from "./useComposerCommands";

export function commandOptionId(listId: string, item: CommandItem): string {
  return `${listId}-${item.command.id}`;
}

export function CommandIcon({ icon }: { icon: ComposerCommand["icon"] }) {
  return icon === "gif" ? (
    <span className={css.gifGlyph}>GIF</span>
  ) : (
    <Icon name={icon} size={16} />
  );
}

/** The name with the part that answered the query marked. */
function Marked({ text, from, to }: { text: string; from: number; to: number }) {
  if (from === to) return <>{text}</>;
  return (
    <>
      {text.slice(0, from)}
      <mark>{text.slice(from, to)}</mark>
      {text.slice(to)}
    </>
  );
}

function selectedRowIn(list: HTMLElement | null): HTMLElement | null {
  return list?.querySelector<HTMLElement>('[aria-selected="true"]') ?? null;
}

/** Stands the hint beside the selected row, kept within the list's height. */
function placeHint(list: HTMLElement | null, card: HTMLElement | null, hint: HTMLElement | null) {
  const row = selectedRowIn(list);
  if (row === null || card === null || hint === null) return;
  const room = card.offsetHeight - hint.offsetHeight;
  const top = row.getBoundingClientRect().top - card.getBoundingClientRect().top;
  const shift = room <= 0 ? 0 : Math.max(0, Math.min(top, room)) - room;
  hint.style.setProperty("--hint-shift", `${shift}px`);
}

export interface CommandListProps {
  /** The list and its hint together, for telling a press on them from one elsewhere. */
  ref?: Ref<HTMLDivElement>;
  id: string;
  hintId: string;
  items: readonly CommandItem[];
  grouped: boolean;
  selectedIndex: number;
  onChoose(item: CommandItem): void;
  onHover(index: number): void;
  t: Translator;
}

/**
 * The commands over the composer, with the hint for the selected one beside
 * them. Focus never leaves the field: the list is the field's combobox popup,
 * as the people after "@" are, so pressing a row must not blur it.
 */
export function CommandList({
  ref,
  id,
  hintId,
  items,
  grouped,
  selectedIndex,
  onChoose,
  onHover,
  t,
}: CommandListProps) {
  const card = useRef<HTMLDivElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const hint = useRef<HTMLDivElement>(null);
  const selected = items[selectedIndex];
  // The hint is drawn anew for each command, so it is placed again for each.
  const selectedId = selected?.command.id;

  // Only a new selection scrolls: the list scrolled by hand stays where it was
  // put until the arrows move on.
  useLayoutEffect(() => {
    selectedRowIn(list.current)?.scrollIntoView?.({ block: "nearest" });
  }, [selectedIndex]);

  useLayoutEffect(() => {
    placeHint(list.current, card.current, hint.current);
  }, [selectedId, items.length]);

  const row = (item: CommandItem, index: number) => {
    const on = index === selectedIndex;
    const { command, match } = item;
    return (
      <button
        key={command.id}
        id={commandOptionId(id, item)}
        type="button"
        role="option"
        aria-selected={on}
        aria-disabled={item.blocked === undefined ? undefined : true}
        aria-describedby={on ? hintId : undefined}
        className={css.row}
        onMouseDown={(event) => event.preventDefault()}
        onPointerMove={() => {
          if (!on) onHover(index);
        }}
        onClick={() => onChoose(item)}
      >
        {/* The spaces between the parts are for the option's name, which a
            screen reader would otherwise read as one run-on word; the row's
            gap does the spacing on screen. */}
        <span className={css.icon} aria-hidden="true">
          <CommandIcon icon={command.icon} />
        </span>{" "}
        <span className={css.name}>
          /{match.via === "name" ? <Marked text={item.name} from={match.from} to={match.to} /> : item.name}
        </span>{" "}
        {match.via === "alias" ? (
          <>
            <span className={css.alias}>
              /<Marked text={command.alias} from={match.from} to={match.to} />
            </span>{" "}
          </>
        ) : null}
        <span className={css.summary}>{t(command.summary)}</span>
        {command.args.length === 0 ? null : (
          <>
            {" "}
            <span className={css.argList}>{command.args.map((arg) => t(arg.label)).join(", ")}</span>
          </>
        )}
      </button>
    );
  };

  let body: ReactNode;
  if (grouped) {
    body = COMMAND_GROUPS.map((group) => {
      const rows = items.flatMap((item, index) =>
        item.command.group === group.id ? [row(item, index)] : [],
      );
      if (rows.length === 0) return null;
      const captionId = `${id}-${group.id}`;
      return (
        <div key={group.id} role="presentation">
          {/* Hidden from reading: it is the group's name, read with the group. */}
          <div id={captionId} className={css.caption} aria-hidden="true">
            {t(group.label)}
          </div>
          <div role="group" aria-labelledby={captionId}>
            {rows}
          </div>
        </div>
      );
    });
  } else {
    body = items.map(row);
  }

  return (
    <div ref={ref} className={css.pop}>
      <div ref={card} className={css.card}>
        {grouped ? null : (
          <div className={css.caption} aria-hidden="true">
            {t("composer.commands.list")} <span>{items.length}</span>
          </div>
        )}
        <div
          ref={list}
          id={id}
          className={css.list}
          role="listbox"
          aria-label={t("composer.commands.list")}
          onScroll={() => placeHint(list.current, card.current, hint.current)}
        >
          {body}
        </div>
        <div className={css.keys} aria-hidden="true">
          <span>
            <kbd>↑</kbd>
            <kbd>↓</kbd> {t("composer.commands.keySelect")}
          </span>
          <span>
            <kbd>Enter</kbd> {t("composer.commands.keyUse")}
          </span>
          <span>
            <kbd>Tab</kbd> {t("composer.commands.keyComplete")}
          </span>
          <span>
            <kbd>Esc</kbd> {t("composer.commands.keyClose")}
          </span>
        </div>
      </div>
      {selected === undefined ? null : (
        <CommandHint key={selected.command.id} ref={hint} id={hintId} item={selected} t={t} />
      )}
    </div>
  );
}

interface CommandHintProps {
  id: string;
  item: CommandItem;
  ref?: Ref<HTMLDivElement>;
  t: Translator;
}

/** What the selected command does, what it takes, an example and where it works. */
function CommandHint({ id, item, ref, t }: CommandHintProps) {
  const { command } = item;
  return (
    <div ref={ref} id={id} className={css.hint}>
      <p className={css.syntax}>
        /{item.name}
        {command.args.map((arg) => (
          <span key={arg.id}>{t(arg.label)}</span>
        ))}
      </p>
      <p>{t(command.detail)}</p>
      {command.args.length === 0 ? null : (
        <dl>
          {command.args.map((arg) => (
            <div key={arg.id}>
              <dt>{t(arg.label)}</dt>
              <dd>
                {arg.optional
                  ? t("composer.commands.optionalHint", { hint: t(arg.hint) })
                  : t(arg.hint)}
              </dd>
            </div>
          ))}
        </dl>
      )}
      {command.example === undefined ? null : (
        <p className={css.example}>
          {t("composer.commands.forExample")} <code>{t(command.example)}</code>
        </p>
      )}
      <p className={css.where}>{t(command.where)}</p>
      {item.blocked === undefined ? null : <p className={css.blocked}>{t(item.blocked)}</p>}
    </div>
  );
}

export interface ArgumentHintProps {
  id: string;
  label: string;
  optional: boolean;
  hint: string;
  /** Why what was typed cannot be used; the command waits until it can. */
  problem?: string;
  t: Translator;
}

/** The hint while an argument is being filled in: its name and what goes in it. */
export function ArgumentHint({ id, label, optional, hint, problem, t }: ArgumentHintProps) {
  return (
    <div className={css.pop}>
      <div id={id} className={`${css.hint} ${css.argumentHint}`}>
        <p className={css.syntax}>
          {label}
          {optional ? <span>{t("composer.commands.optional")}</span> : null}
        </p>
        <p>{hint}</p>
        {problem === undefined ? null : <p className={css.blocked}>{problem}</p>}
      </div>
    </div>
  );
}
