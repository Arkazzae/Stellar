import type { PinnedMessageVM } from "@/components/room/panels/channelPanelModel";
import { toParts, type MentionTarget } from "@/lib/messages/parts";
import type { TranscriptSectionVM } from "@/components/room/transcript/Transcript";
import type { InterfaceLocale } from "@/i18n";
import { messageReference, type TranscriptSection } from "@/lib/format/format";
import type { Message, Reaction, Server, User } from "@/ports";

/** The one author id the transcript renders as a server notice. */
export const SYSTEM_AUTHOR_ID = "system";

/** Everything outside the messages that decides how a transcript reads. */
export interface PresentationContext {
  /** Whose blocks get the owner's tools; undefined while nobody is signed in. */
  selfId: string | undefined;
  server: Pick<Server, "name" | "iconUrl">;
  /** Author id to picture. A message carries a name, never a face. */
  faces: ReadonlyMap<string, string>;
  customEmoji: ReadonlyMap<string, string>;
  mentions: ReadonlyMap<string, MentionTarget>;
  pinnedIds: ReadonlySet<string>;
  reactionsOf: (messageId: string) => Reaction[];
  /** The badge of a file with no extension, in the reader's language. */
  extFallback: string;
}

/**
 * Turns one transcript's grouped messages into what it draws.
 *
 * A message carries an id and a name for its author, so the directory-side
 * facts (pictures, who you are, which messages are pinned) are merged in here
 * and the components stay presentational.
 *
 * This is the stateless form, for a one-off presentation. Providers use a
 * `TranscriptPresenter` so that an arrival does not rebuild the history.
 */
export function presentTranscript(
  transcript: readonly TranscriptSection[],
  context: PresentationContext,
): TranscriptSectionVM[] {
  return presentSections(transcript, context, undefined);
}

/** One channel's, conversation's or thread's groups, presented incrementally. */
export type TranscriptPresenter = (
  transcript: readonly TranscriptSection[],
  context: PresentationContext,
) => TranscriptSectionVM[];

/**
 * Creates a presenter that keeps the previous view model between calls.
 *
 * Parts are parsed again only when the `Message` object is replaced or one of
 * the parsing inputs (custom emoji, mentions, the file fallback) changes. The
 * fields that can change without a new message — reactions, pins, names,
 * pictures and server identity — are derived on every call, and a body, block
 * or day is reused only when all of its fields still match.
 *
 * One instance belongs to one mounted transcript: it retains only the entries
 * of the transcript it was last given, so switching channel or conversation
 * releases the previous one. A provider creates its instance once and passes
 * the real `buildTranscript` result on every update.
 */
export function createTranscriptPresenter(): TranscriptPresenter {
  const cache: TranscriptCache = {
    bodies: new Map(),
    groups: new Map(),
    sections: new Map(),
    last: undefined,
  };
  return (transcript, context) => presentSections(transcript, context, cache);
}

type GroupVM = TranscriptSectionVM["groups"][number];
type BodyVM = GroupVM["bodies"][number];
type PartsVM = BodyVM["parts"];

interface CachedBody {
  /** The parsing inputs these parts were made from. */
  emoji: ReadonlyMap<string, string>;
  mentions: ReadonlyMap<string, MentionTarget>;
  extFallback: string;
  parts: PartsVM;
  reactions: Reaction[];
  pinned: boolean;
  body: BodyVM;
}

interface TranscriptCache {
  /** Keyed by the immutable message object: a replacement object is a new body. */
  bodies: Map<Message, CachedBody>;
  groups: Map<string, GroupVM>;
  sections: Map<string, TranscriptSectionVM>;
  /** The array last returned, when the sections in it did not change. */
  last: TranscriptSectionVM[] | undefined;
}

/** Same items in the same order, compared by reference. */
function sameItems<T>(left: readonly T[], right: readonly T[]): boolean {
  if (left === right) return true;
  if (left.length !== right.length) return false;
  for (let index = 0; index < left.length; index += 1) {
    if (left[index] !== right[index]) return false;
  }
  return true;
}

/** Reaction aggregates are copied on every read, so compare their values. */
function sameReactions(
  left: readonly Reaction[],
  right: readonly Reaction[],
): boolean {
  if (left === right) return true;
  if (left.length !== right.length) return false;
  for (let index = 0; index < left.length; index += 1) {
    const before = left[index];
    const after = right[index];
    if (before === undefined || after === undefined) return false;
    if (
      before.emoji !== after.emoji ||
      before.count !== after.count ||
      before.mine !== after.mine
    ) {
      return false;
    }
  }
  return true;
}

/**
 * Whether two view models were built with the same fields.
 *
 * Shallow rather than a list of known fields: a field added to a block later
 * becomes part of the answer without a second place to remember it.
 */
function sameFields(left: object, right: object): boolean {
  const a = left as Readonly<Record<string, unknown>>;
  const b = right as Readonly<Record<string, unknown>>;
  const keys = Object.keys(a);
  if (keys.length !== Object.keys(b).length) return false;
  return keys.every((key) => Object.hasOwn(b, key) && Object.is(a[key], b[key]));
}

function presentBody(
  message: Message,
  context: PresentationContext,
  cache: TranscriptCache | undefined,
): BodyVM {
  const { customEmoji, mentions, extFallback, pinnedIds, reactionsOf } = context;
  const previous = cache?.bodies.get(message);
  const sameParsing =
    previous !== undefined &&
    previous.emoji === customEmoji &&
    previous.mentions === mentions &&
    previous.extFallback === extFallback;
  const parts = previous !== undefined && sameParsing
    ? previous.parts
    : toParts(message, {
        emoji: customEmoji,
        mentions,
        extFallback,
      });
  const pinned = pinnedIds.has(message.id);
  const reactions = reactionsOf(message.id);

  if (
    previous !== undefined &&
    sameParsing &&
    previous.pinned === pinned &&
    sameReactions(previous.reactions, reactions)
  ) {
    return previous.body;
  }

  const body: BodyVM = {
    id: message.id,
    parts,
    ...(pinned ? { pinned: true } : {}),
    ...(reactions.length > 0 ? { reactions } : {}),
  };
  cache?.bodies.set(message, {
    emoji: customEmoji,
    mentions,
    extFallback,
    parts,
    reactions,
    pinned,
    body,
  });
  return body;
}

function presentGroup(
  group: TranscriptSection["groups"][number],
  context: PresentationContext,
  bodies: BodyVM[],
  cache: TranscriptCache | undefined,
): GroupVM {
  const { selfId, server, faces } = context;
  const system = group.authorId === SYSTEM_AUTHOR_ID;
  const face = system ? server.iconUrl : faces.get(group.authorId);
  // A reply always starts its own block, so the head message is the one that
  // answers something.
  const answered = group.messages[0]?.replyTo;
  const candidate: GroupVM = {
    id: group.key,
    ...(answered === undefined || answered === null ? {} : { replyTo: answered }),
    authorId: group.authorId,
    authorName: system ? server.name : group.authorName,
    ...(selfId !== undefined && group.authorId === selfId ? { own: true } : {}),
    ...(face === undefined ? {} : { authorAvatarUrl: face }),
    ...(system ? { system: true } : {}),
    timestamp: group.startedAt,
    // One entry per message: a block can hold several, and each of them is
    // separately editable and deletable.
    bodies,
  };

  const previous = cache?.groups.get(group.key);
  if (previous === undefined) {
    cache?.groups.set(group.key, candidate);
    return candidate;
  }

  // Keep the old bodies array too when its items are unchanged, so both the
  // group and its list stay stable through an unrelated arrival.
  const next = sameItems(previous.bodies, bodies)
    ? { ...candidate, bodies: previous.bodies }
    : candidate;
  if (sameFields(previous, next)) return previous;
  cache?.groups.set(group.key, next);
  return next;
}

function presentSections(
  transcript: readonly TranscriptSection[],
  context: PresentationContext,
  cache: TranscriptCache | undefined,
): TranscriptSectionVM[] {
  const presented: TranscriptSectionVM[] = [];
  const activeMessages = new Set<Message>();
  const activeGroups = new Set<string>();
  const activeDays = new Set<string>();

  for (const section of transcript) {
    activeDays.add(section.day);
    const groups: GroupVM[] = [];
    for (const group of section.groups) {
      activeGroups.add(group.key);
      const bodies: BodyVM[] = [];
      for (const message of group.messages) {
        activeMessages.add(message);
        bodies.push(presentBody(message, context, cache));
      }
      groups.push(presentGroup(group, context, bodies, cache));
    }

    const previous = cache?.sections.get(section.day);
    const candidate: TranscriptSectionVM = { day: section.day, groups };
    const next =
      previous !== undefined && sameItems(previous.groups, groups)
        ? { ...candidate, groups: previous.groups }
        : candidate;
    const sectionVM =
      previous !== undefined && sameFields(previous, next) ? previous : next;
    cache?.sections.set(section.day, sectionVM);
    presented.push(sectionVM);
  }

  if (cache === undefined) return presented;

  // A transcript is the complete set of what this presenter is showing. Any
  // entry outside it belongs to a channel or conversation no longer open.
  keepOnly(cache.bodies, activeMessages);
  keepOnly(cache.groups, activeGroups);
  keepOnly(cache.sections, activeDays);

  const result =
    cache.last !== undefined && sameItems(cache.last, presented)
      ? cache.last
      : presented;
  cache.last = result;
  return result;
}

function keepOnly<K, V>(map: Map<K, V>, active: ReadonlySet<K>): void {
  for (const key of map.keys()) {
    if (!active.has(key)) map.delete(key);
  }
}

/** Member id to picture, for the members that have one. */
export function memberFaces(members: readonly User[]): Map<string, string> {
  const faces = new Map<string, string>();
  for (const member of members) {
    if (member.avatarUrl !== undefined) faces.set(member.id, member.avatarUrl);
  }
  return faces;
}

/** Lower-cased username to the member an `@name` resolves to. */
export function mentionTargets(
  members: readonly User[],
  selfId: string | undefined,
): Map<string, MentionTarget> {
  return new Map(
    members.map((member) => [
      member.username.toLocaleLowerCase(),
      {
        userId: member.id,
        username: member.username,
        displayName: member.displayName,
        self: member.id === selfId,
      },
    ]),
  );
}

/** The pinned list's rows, in the order the messages were given. */
export function pinnedRows(
  pinned: readonly Message[],
  locale: InterfaceLocale,
  serverName: string,
): PinnedMessageVM[] {
  return pinned.flatMap((message) => {
    // Only ever absent if a server lists a message it does not consider
    // pinned. Dropping it beats dating a row with the epoch.
    if (message.pinnedAt === undefined) return [];
    const { authorName, preview } = messageReference(message, locale);
    const system = message.author.id === SYSTEM_AUTHOR_ID;
    return [{
      id: message.id,
      authorId: message.author.id,
      authorName: system ? serverName : authorName,
      preview,
      timestamp: message.createdAt,
      pinnedAt: message.pinnedAt,
    }];
  });
}
