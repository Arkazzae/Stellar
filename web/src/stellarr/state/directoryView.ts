import type {
  PersonVM,
  SectionVM,
  TextChannelVM,
  VoiceChannelVM,
} from "@/components/view-models";
import { initials } from "@/lib/format/format";
import type {
  Channel,
  Section,
  Server,
  ServerPermission,
  User,
  VoiceOccupant,
} from "@/ports";

/** What the directory ports hold at one moment, read in one go. */
export interface DirectorySnapshot {
  server: Server;
  servers: Server[];
  channels: Channel[];
  sections: Section[];
  members: User[];
  permissions: ServerPermission[];
  occupancy: ReadonlyMap<string, VoiceOccupant[]>;
}

/** The directory as the shell shows it: view models and permission flags. */
export interface DirectoryView {
  server: Server;
  servers: Server[];
  /** Two-letter mark for the settings brand and the invite card. */
  serverMark: string;
  channels: Channel[];
  textChannels: TextChannelVM[];
  voiceChannels: VoiceChannelVM[];
  /** Collapsible rail groups, in display order. */
  sections: SectionVM[];
  members: User[];
  permissions: ServerPermission[];
  /** Every member as a person, presence included, in bootstrap order. */
  people: PersonVM[];
  /** Who the rail shows under one voice channel. */
  occupantsOf: (channelId: string) => PersonVM[];
  /** Everyone on any voice channel, each person once. */
  inVoice: PersonVM[];
  /** At least one permission that a server-settings pane represents. */
  canAccessServerSettings: boolean;
  canManageServer: boolean;
  canManageChannels: boolean;
  canManageMessages: boolean;
  canModerateMembers: boolean;
  canManageBans: boolean;
  /** The channel the room falls back to before anything is picked. */
  firstTextChannelId: string | null;
  channelName: (channelId: string | null) => string;
}

const NO_OCCUPANTS: VoiceOccupant[] = [];
const NO_PEOPLE: PersonVM[] = [];

/**
 * The permissions a server-settings pane is for. Moderating messages and
 * members happens in the room, and what every member may do, such as sending
 * messages or joining voice, opens nothing; an administrator holds these too.
 */
const SETTINGS_PERMISSIONS: ReadonlySet<ServerPermission> = new Set<ServerPermission>([
  "manageServer",
  "manageChannels",
  "manageRoles",
  "manageMembers",
  "manageBans",
  "viewAuditLog",
]);

/**
 * Whether two occupancy maps say the same thing.
 *
 * `PresencePort.occupantsOf` returns a fresh `[]` for an empty channel, so an
 * identity compare would report a change on every read and
 * `useSyncExternalStore` would loop. Two empty lists are the same answer.
 */
export function sameOccupancy(
  a: ReadonlyMap<string, VoiceOccupant[]>,
  b: ReadonlyMap<string, VoiceOccupant[]>,
): boolean {
  if (a.size !== b.size) return false;
  for (const [channelId, list] of a) {
    const other = b.get(channelId);
    if (other === undefined) return false;
    if (other !== list && (list.length > 0 || other.length > 0)) return false;
  }
  return true;
}

/** "Orbit" gives "OR" and "Nocna zmiana" gives "NZ"; an empty name gives "ST". */
export function serverMark(name: string): string {
  return name.trim() === "" ? "ST" : initials(name);
}

function toPerson(user: User): PersonVM {
  return {
    id: user.id,
    displayName: user.displayName,
    initials: initials(user.displayName),
    presence: user.presence,
    ...(user.avatarUrl === undefined ? {} : { avatarUrl: user.avatarUrl }),
  };
}

/**
 * A `VoiceOccupant` carries what signalling knows about someone on a channel:
 * an id, a name and a microphone. Their face has to come from the directory,
 * and a peer it has never heard of still renders, on initials, mid-join.
 */
function occupantToPerson(
  occupant: VoiceOccupant,
  members: ReadonlyMap<string, User>,
): PersonVM {
  const avatarUrl = members.get(occupant.userId)?.avatarUrl;
  return {
    id: occupant.userId,
    displayName: occupant.displayName,
    initials: initials(occupant.displayName),
    // Someone in a call is reachable by definition, and the row shows their
    // microphone in place of their presence anyway.
    presence: "online",
    muted: occupant.muted,
    ...(occupant.sharing === true ? { sharing: true } : {}),
    ...(occupant.camera === true ? { camera: true } : {}),
    ...(avatarUrl === undefined ? {} : { avatarUrl }),
  };
}

export function deriveTextChannels(channels: readonly Channel[]): TextChannelVM[] {
  return channels
    .filter((channel) => channel.kind === "text")
    .map((channel) => ({
      id: channel.id,
      name: channel.name,
      ...(channel.topic === undefined ? {} : { topic: channel.topic }),
      ...(channel.sectionId === undefined ? {} : { sectionId: channel.sectionId }),
    }));
}

/** `next` itself, or `previous` when both hold the very same items in the same order. */
function sameItems<T>(previous: readonly T[] | undefined, next: T[]): T[] {
  if (previous === undefined || previous.length !== next.length) return next;
  return next.every((item, index) => item === previous[index]) ? (previous as T[]) : next;
}

function sameOccupantVM(a: PersonVM, b: PersonVM): boolean {
  return (
    a.id === b.id &&
    a.displayName === b.displayName &&
    a.muted === b.muted &&
    a.sharing === b.sharing &&
    a.camera === b.camera &&
    a.avatarUrl === b.avatarUrl
  );
}

function sameVoiceChannel(a: VoiceChannelVM, b: VoiceChannelVM): boolean {
  return (
    a.id === b.id &&
    a.name === b.name &&
    a.sectionId === b.sectionId &&
    a.userLimit === b.userLimit &&
    a.occupants === b.occupants
  );
}

export type DirectoryViewDeriver = (
  snapshot: DirectorySnapshot,
  textChannels?: TextChannelVM[],
) => DirectoryView;

/**
 * Derives the view, handing back every part that says the same as last time
 * as the very object it was: a person, an occupant, a voice channel and the
 * lists of them. A hub frame always brings fresh occupant objects, so seats
 * are compared by what they say; members are replaced one at a time, so a
 * person is kept for as long as their `User` is. Whatever a row is keyed on
 * stays put through a presence change or someone else's seat moving.
 */
export function createDirectoryViewDeriver(): DirectoryViewDeriver {
  const people = new WeakMap<User, PersonVM>();
  let last: DirectoryView | null = null;
  let lastSections: { from: readonly Section[]; view: SectionVM[] } | null = null;
  /** Each channel's occupants as last shown, by channel id. */
  let lastOccupants = new Map<string, PersonVM[]>();

  const personOf = (user: User): PersonVM => {
    let person = people.get(user);
    if (person === undefined) {
      person = toPerson(user);
      people.set(user, person);
    }
    return person;
  };

  return (snapshot, textChannels = deriveTextChannels(snapshot.channels)) => {
    const { server, channels, members, permissions, occupancy } = snapshot;
    const previous = last;
    const membersById = new Map(members.map((member) => [member.id, member]));

    const occupants = new Map<string, PersonVM[]>();
    const peopleOn = (channelId: string): PersonVM[] => {
      const before = lastOccupants.get(channelId);
      const shown = (occupancy.get(channelId) ?? NO_OCCUPANTS).map((occupant, index) => {
        const person = occupantToPerson(occupant, membersById);
        const kept = before?.[index];
        return kept !== undefined && sameOccupantVM(kept, person) ? kept : person;
      });
      return sameItems(before, shown);
    };

    const previousChannels = new Map(
      (previous?.voiceChannels ?? []).map((channel) => [channel.id, channel]),
    );
    const voiceChannels = sameItems(
      previous?.voiceChannels,
      channels
        .filter((channel) => channel.kind === "voice")
        .map((channel) => {
          const seated = peopleOn(channel.id);
          occupants.set(channel.id, seated);
          const next: VoiceChannelVM = {
            id: channel.id,
            name: channel.name,
            occupants: seated,
            ...(channel.sectionId === undefined ? {} : { sectionId: channel.sectionId }),
            // The server reads 0 as no limit.
            ...(channel.userLimit === undefined || channel.userLimit === 0
              ? {}
              : { userLimit: channel.userLimit }),
          };
          const kept = previousChannels.get(channel.id);
          return kept !== undefined && sameVoiceChannel(kept, next) ? kept : next;
        }),
    );
    lastOccupants = occupants;

    const sameVoice = previous !== null && previous.voiceChannels === voiceChannels;
    let inVoice: PersonVM[];
    let occupantsOf: (channelId: string) => PersonVM[];
    if (sameVoice) {
      ({ inVoice, occupantsOf } = previous);
    } else {
      const seen = new Map<string, PersonVM>();
      for (const channel of voiceChannels) {
        for (const person of channel.occupants) {
          if (!seen.has(person.id)) seen.set(person.id, person);
        }
      }
      inVoice = sameItems(previous?.inVoice, [...seen.values()]);
      occupantsOf = (channelId) => occupants.get(channelId) ?? NO_PEOPLE;
    }

    if (lastSections?.from !== snapshot.sections) {
      lastSections = {
        from: snapshot.sections,
        view: sameItems(
          lastSections?.view,
          snapshot.sections.map((section) => ({ id: section.id, name: section.name })),
        ),
      };
    }

    const view: DirectoryView = {
      server,
      servers: snapshot.servers,
      serverMark: serverMark(server.name),
      channels,
      textChannels,
      voiceChannels,
      sections: lastSections.view,
      members,
      permissions,
      people: sameItems(previous?.people, members.map(personOf)),
      occupantsOf,
      inVoice,
      canAccessServerSettings: permissions.some((permission) => SETTINGS_PERMISSIONS.has(permission)),
      canManageServer: permissions.includes("manageServer"),
      canManageChannels: permissions.includes("manageChannels"),
      canManageMessages: permissions.includes("manageMessages"),
      canModerateMembers: permissions.includes("moderateMembers"),
      canManageBans: permissions.includes("manageBans"),
      firstTextChannelId: textChannels[0]?.id ?? null,
      channelName: (channelId) =>
        channels.find((channel) => channel.id === channelId)?.name ?? "",
    };
    last = view;
    return view;
  };
}

/**
 * `textChannels` lets a caller pass the list it already derived from this
 * snapshot's `channels`, so that list keeps its identity while only occupancy
 * or presence moves.
 */
export function deriveDirectoryView(
  snapshot: DirectorySnapshot,
  textChannels?: TextChannelVM[],
): DirectoryView {
  return createDirectoryViewDeriver()(snapshot, textChannels);
}

/**
 * Splices one group's new order back into the server-wide order.
 *
 * The channel endpoint takes a single order for the whole server, while a drag
 * reorders one rail group. The existing order is walked in place, and wherever
 * a slot belongs to the dragged group it takes the next id from the new
 * sequence. Every channel outside the group keeps the position it had. A
 * channel dropped in from another group brings its old slot along, which is
 * all the group it left needs: its own channels keep their order.
 */
export function spliceGroupOrder(
  channels: readonly Channel[],
  newGroupOrder: readonly string[],
): string[] {
  const group = new Set(newGroupOrder);
  let cursor = 0;
  return channels.map((channel) =>
    group.has(channel.id) ? (newGroupOrder[cursor++] ?? channel.id) : channel.id,
  );
}
