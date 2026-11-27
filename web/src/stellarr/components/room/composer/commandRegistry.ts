import type { IconName } from "@stellarr/icons";
import { BAN_REASON_MAX_LENGTH } from "@/components/confirmations";
import type { MessageKey } from "@/i18n";
import type { MentionCandidate } from "@/lib/messages/composerMentions";
import { POLL_LIMITS } from "@/ports/polls";

import { GIF_QUERY_MAX_LENGTH } from "./GifBrowser";

/*
 * Every command the composer offers, in the order the list shows them.
 *
 * A command is a way into something the app already does: it names the
 * feature, says when it is there, and opens it. Whether it is there is asked
 * of the context each time, because the same composer sits in a channel, a
 * thread and a conversation, and a command whose feature is missing (or is
 * forbidden to the person) is not listed at all.
 *
 * A new command is one more entry here plus its strings in the three packs.
 */

export type CommandId =
  | "file"
  | "poll"
  | "gif"
  | "voice"
  | "schedule"
  | "code"
  | "spoiler"
  | "pinned"
  | "status"
  | "kick"
  | "ban";

export type CommandGroup = "add" | "message" | "server";

/**
 * Where the phone's sheet puts a command: a tile across the top, in the order
 * the numbers give, a row in its group, or nowhere, for one whose button
 * stays in the field's row.
 */
export type SheetPlace = { tile: number } | "row" | "none";

/** The groups in the order the list draws them, each with its caption. */
export const COMMAND_GROUPS: readonly { id: CommandGroup; label: MessageKey }[] = [
  { id: "add", label: "composer.commands.group.add" },
  { id: "message", label: "composer.commands.group.message" },
  { id: "server", label: "composer.commands.group.server" },
];

export interface CommandArgument {
  id: string;
  /** A person is picked from the same list that "@" opens; text is typed. */
  kind: "text" | "person";
  optional: boolean;
  label: MessageKey;
  /** One sentence: what goes in and where it ends up. */
  hint: MessageKey;
  /** As long as the feature that receives it accepts. */
  maxLength?: number;
  /**
   * Why what was typed cannot be used, or nothing when it can. The command
   * does not run while an argument has a reason against it.
   */
  check?: (value: string, context: CommandCheckContext) => MessageKey | undefined;
}

/** What checking an argument may read: only what the reading of a value needs. */
export interface CommandCheckContext {
  /** Reads a typed send time, where messages can be scheduled. */
  parseWhen?: (text: string) => { at: Date } | { problem: MessageKey };
}

/**
 * What the commands can reach from where the composer is. A capability that
 * is missing hides the command that needs it; a reason given for one that is
 * there keeps the command listed but not runnable, so the hint can say why.
 */
export interface CommandContext {
  openFilePicker?: () => void;
  fileBlocked?: MessageKey;
  openGifs?: (query: string) => void;
  /** Opens a new poll with the question typed; only where polls can be sent. */
  openPoll?: (question: string) => void;
  recordVoice?: () => void;
  voiceBlocked?: MessageKey;
  enterCode: (language: string) => void;
  insertSpoiler: (words: string) => void;
  openPinned?: () => void;
  openStatus?: () => void;
  /** Sending later, where the message can be scheduled. */
  schedule?: {
    /** Reads a typed time: the instant, or why it is not one. */
    parse(text: string): { at: Date } | { problem: MessageKey };
    /** Puts the time in the field as the message's send time. */
    choose(at: Date): void;
    openPicker(): void;
  };
  /** Asks first: running the command never removes anyone by itself. */
  kick?: (person: MentionCandidate) => void;
  /** Asks first, with the reason already in the question. */
  ban?: (person: MentionCandidate, reason: string) => void;
}

/** The arguments a command was given, by id. */
export interface CommandInput {
  text(id: string): string;
  person(id: string): MentionCandidate | undefined;
}

export type Availability = { shown: false } | { shown: true; blocked?: MessageKey };

export interface ComposerCommand {
  id: CommandId;
  group: CommandGroup;
  /** The name in the reader's language, without the slash. */
  name: MessageKey;
  /** The English name, which every language accepts. */
  alias: string;
  /** What the phone's sheet calls it, in a sentence's case. */
  label: MessageKey;
  /** The short line beside the name. */
  summary: MessageKey;
  /** What it does, for the hint. */
  detail: MessageKey;
  /** A whole command as one would type it; only commands that take words have one. */
  example?: MessageKey;
  where: MessageKey;
  icon: IconName | "gif";
  sheet: SheetPlace;
  args: readonly CommandArgument[];
  available(context: CommandContext): Availability;
  run(context: CommandContext, input: CommandInput): void;
}

const READY: Availability = { shown: true };
const HIDDEN: Availability = { shown: false };

function shownUnless(reason: MessageKey | undefined): Availability {
  return reason === undefined ? READY : { shown: true, blocked: reason };
}

/** The strings every command has, named after its id. */
function copy<Id extends CommandId>(id: Id) {
  return {
    name: `composer.command.${id}.name`,
    label: `composer.command.${id}.label`,
    summary: `composer.command.${id}.summary`,
    detail: `composer.command.${id}.detail`,
    where: `composer.command.${id}.where`,
  } as const;
}

function argument<Id extends CommandId, Arg extends string>(
  command: Id,
  id: Arg,
  shape: {
    kind: CommandArgument["kind"];
    optional: boolean;
    maxLength?: number;
    check?: CommandArgument["check"];
  },
) {
  return {
    id,
    ...shape,
    label: `composer.command.${command}.arg.${id}`,
    hint: `composer.command.${command}.arg.${id}.hint`,
  } as const;
}

export const COMMANDS: readonly ComposerCommand[] = [
  {
    id: "file",
    group: "add",
    alias: "file",
    ...copy("file"),
    icon: "paperclip",
    sheet: { tile: 1 },
    args: [],
    available: (context) =>
      context.openFilePicker === undefined ? HIDDEN : shownUnless(context.fileBlocked),
    run: (context) => context.openFilePicker?.(),
  },
  {
    id: "poll",
    group: "add",
    alias: "poll",
    ...copy("poll"),
    example: "composer.command.poll.example",
    icon: "chart",
    sheet: { tile: 2 },
    args: [argument("poll", "question", { kind: "text", optional: true, maxLength: POLL_LIMITS.question })],
    available: (context) => (context.openPoll === undefined ? HIDDEN : READY),
    run: (context, input) => context.openPoll?.(input.text("question").trim()),
  },
  {
    id: "gif",
    group: "add",
    alias: "gif",
    ...copy("gif"),
    example: "composer.command.gif.example",
    icon: "gif",
    // Back among the rows once events take their tile.
    sheet: { tile: 4 },
    args: [argument("gif", "query", { kind: "text", optional: true, maxLength: GIF_QUERY_MAX_LENGTH })],
    available: (context) => (context.openGifs === undefined ? HIDDEN : READY),
    run: (context, input) => context.openGifs?.(input.text("query")),
  },
  {
    id: "voice",
    group: "add",
    alias: "voice",
    ...copy("voice"),
    icon: "mic",
    // The microphone stays in the phone's row.
    sheet: "none",
    args: [],
    available: (context) =>
      context.recordVoice === undefined ? HIDDEN : shownUnless(context.voiceBlocked),
    run: (context) => context.recordVoice?.(),
  },
  {
    id: "schedule",
    group: "message",
    alias: "schedule",
    ...copy("schedule"),
    example: "composer.command.schedule.example",
    icon: "clock",
    sheet: { tile: 3 },
    args: [
      argument("schedule", "when", {
        kind: "text",
        optional: true,
        check: (value, context) => {
          if (value.trim() === "" || context.parseWhen === undefined) return undefined;
          const parsed = context.parseWhen(value);
          return "problem" in parsed ? parsed.problem : undefined;
        },
      }),
    ],
    available: (context) => (context.schedule === undefined ? HIDDEN : READY),
    run: (context, input) => {
      const when = input.text("when").trim();
      if (context.schedule === undefined) return;
      if (when === "") {
        context.schedule.openPicker();
        return;
      }
      const parsed = context.schedule.parse(when);
      if ("at" in parsed) context.schedule.choose(parsed.at);
    },
  },
  {
    id: "code",
    group: "message",
    alias: "code",
    ...copy("code"),
    example: "composer.command.code.example",
    icon: "code",
    sheet: "row",
    args: [argument("code", "language", { kind: "text", optional: true })],
    available: () => READY,
    run: (context, input) => context.enterCode(input.text("language")),
  },
  {
    id: "spoiler",
    group: "message",
    alias: "spoiler",
    ...copy("spoiler"),
    example: "composer.command.spoiler.example",
    icon: "spoiler",
    sheet: "row",
    args: [argument("spoiler", "text", { kind: "text", optional: true })],
    available: () => READY,
    run: (context, input) => context.insertSpoiler(input.text("text")),
  },
  {
    id: "pinned",
    group: "server",
    alias: "pinned",
    ...copy("pinned"),
    icon: "pin",
    sheet: "row",
    args: [],
    available: (context) => (context.openPinned === undefined ? HIDDEN : READY),
    run: (context) => context.openPinned?.(),
  },
  {
    id: "status",
    group: "server",
    alias: "status",
    ...copy("status"),
    icon: "moon",
    sheet: "row",
    args: [],
    available: (context) => (context.openStatus === undefined ? HIDDEN : READY),
    run: (context) => context.openStatus?.(),
  },
  {
    id: "kick",
    group: "server",
    alias: "kick",
    ...copy("kick"),
    example: "composer.command.kick.example",
    icon: "signout",
    sheet: "row",
    // Kicking takes no reason on the server, so the command asks for none.
    args: [argument("kick", "person", { kind: "person", optional: false })],
    available: (context) => (context.kick === undefined ? HIDDEN : READY),
    run: (context, input) => {
      const person = input.person("person");
      if (person !== undefined) context.kick?.(person);
    },
  },
  {
    id: "ban",
    group: "server",
    alias: "ban",
    ...copy("ban"),
    example: "composer.command.ban.example",
    icon: "ban",
    sheet: "row",
    args: [
      argument("ban", "person", { kind: "person", optional: false }),
      argument("ban", "reason", { kind: "text", optional: true, maxLength: BAN_REASON_MAX_LENGTH }),
    ],
    available: (context) => (context.ban === undefined ? HIDDEN : READY),
    run: (context, input) => {
      const person = input.person("person");
      if (person !== undefined) context.ban?.(person, input.text("reason").trim());
    },
  },
];

/** True when every argument the command cannot do without has been given. */
export function argumentsComplete(command: ComposerCommand, input: CommandInput): boolean {
  return command.args.every(
    (arg) =>
      arg.optional ||
      (arg.kind === "person" ? input.person(arg.id) !== undefined : input.text(arg.id).trim() !== ""),
  );
}

/** Each argument's reason against what was typed into it, by argument id. */
export function argumentProblems(
  command: ComposerCommand,
  input: CommandInput,
  context: CommandCheckContext,
): Readonly<Record<string, MessageKey>> {
  const problems: Record<string, MessageKey> = {};
  for (const arg of command.args) {
    const problem = arg.check?.(input.text(arg.id), context);
    if (problem !== undefined) problems[arg.id] = problem;
  }
  return problems;
}
