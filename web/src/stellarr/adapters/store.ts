import type {
  Bootstrap,
  Channel,
  ChannelId,
  ChannelPlacement,
  Presence,
  Section,
  Server,
  ServerPermission,
  User,
  UserId,
  VoiceOccupant,
} from "@/lib/api/types";

/** What the read-only ports show: one server, as `bootstrap` described it. */
export interface DirectorySnapshot {
  server: Server | null;
  servers: Server[];
  channels: Channel[];
  sections: Section[];
  members: User[];
  permissions: ServerPermission[];
  occupancy: Record<ChannelId, VoiceOccupant[]>;
}

const EMPTY: DirectorySnapshot = {
  server: null,
  servers: [],
  channels: [],
  sections: [],
  members: [],
  permissions: [],
  occupancy: {},
};

type Listener = () => void;

/** A part of the directory a change can touch: a field, one room's list or one member. */
type Part =
  | "server"
  | "servers"
  | "channels"
  | "sections"
  | "permissions"
  | `room:${ChannelId}`
  | `member:${UserId}`;

/**
 * The directory every read-only port reads from.
 *
 * `bootstrap` delivers the server, its channels, its members and the voice
 * occupancy in one payload. Adapters share this store instead of each keeping
 * a copy that drifts: the realtime client and the admin adapter write to it,
 * the ports read from it, and React subscribes to it.
 *
 * Every change publishes a new snapshot object, so `useSyncExternalStore` and
 * `React.memo` can compare by identity.
 */
export class DirectoryStore {
  #snapshot: DirectorySnapshot = EMPTY;
  readonly #listeners = new Set<Listener>();
  /** Changes made so far, and the change that last touched each part. */
  #changes = 0;
  readonly #touched = new Map<Part, number>();

  getSnapshot = (): DirectorySnapshot => this.#snapshot;

  subscribe = (listener: Listener): (() => void) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };

  hydrate(bootstrap: Bootstrap): void {
    this.#touched.clear();
    this.#publish({
      server: bootstrap.server,
      servers: bootstrap.servers,
      channels: [...bootstrap.channels].sort(byPosition),
      sections: [...bootstrap.sections].sort(byOrder),
      members: bootstrap.members,
      permissions: bootstrap.permissions,
      occupancy: bootstrap.voice ?? {},
    });
  }

  reset(): void {
    this.#touched.clear();
    this.#publish(EMPTY);
  }

  /**
   * Where the changes stand now. A resync takes it before it asks for the
   * bootstrap, to tell what the hub or this client changed meanwhile.
   */
  mark(): number {
    return this.#changes;
  }

  /**
   * Brings the directory back in step with a bootstrap fetched after the hub
   * came back, since the hub does not replay what it sent while away.
   *
   * Whatever says the same as before stays the very object it was, so only
   * what really changed renders and a drag in progress goes on. A part that
   * changed after `mark`, such as a room the hub sent a list for or a channel
   * order just dropped, keeps what it has: that is newer than the bootstrap.
   * A bootstrap for another server changes nothing.
   */
  resync(bootstrap: Bootstrap, mark: number): void {
    const current = this.#snapshot;
    if (current.server === null || current.server.id !== bootstrap.server.id) return;
    const stale = (part: Part) => (this.#touched.get(part) ?? 0) <= mark;
    const take = <T>(part: Part, now: T, next: T, keep: (now: T, next: T) => T): T =>
      stale(part) ? keep(now, next) : now;

    const voice = bootstrap.voice ?? {};
    let occupancy = current.occupancy;
    for (const channelId of new Set([...Object.keys(current.occupancy), ...Object.keys(voice)])) {
      const now = current.occupancy[channelId];
      if (!stale(`room:${channelId}`)) continue;
      const next = voice[channelId] ?? [];
      if (sameOccupants(now ?? [], next)) continue;
      occupancy = { ...occupancy, [channelId]: next };
    }

    const live = new Map(current.members.map((member) => [member.id, member]));
    const members = keptList(
      current.members,
      bootstrap.members.flatMap((member) => {
        if (stale(`member:${member.id}`)) return [member];
        // Changed here meanwhile: kept as it is now, or gone if it was removed.
        const now = live.get(member.id);
        return now === undefined ? [] : [now];
      }),
    );

    const next: DirectorySnapshot = {
      server: take("server", current.server, bootstrap.server, kept),
      servers: take("servers", current.servers, bootstrap.servers, keptList),
      channels: take("channels", current.channels, [...bootstrap.channels].sort(byPosition), keptList),
      sections: take("sections", current.sections, [...bootstrap.sections].sort(byOrder), keptList),
      members,
      permissions: take("permissions", current.permissions, bootstrap.permissions, keptList),
      occupancy,
    };
    const parts = Object.keys(next) as (keyof DirectorySnapshot)[];
    if (parts.some((part) => next[part] !== current[part])) this.#publish(next);
  }

  /** A presence the directory already shows publishes nothing. */
  setPresence(userId: UserId, presence: Presence): void {
    this.#touch(`member:${userId}`);
    const member = this.#snapshot.members.find((item) => item.id === userId);
    if (member === undefined || member.presence === presence) return;
    this.#replaceMembers((item) => (item === member ? { ...item, presence } : item));
  }

  setUser(user: User): void {
    this.#touch(`member:${user.id}`);
    this.#replaceMembers((member) => (member.id === user.id ? user : member));
  }

  removeMember(userId: UserId): void {
    this.#touch(`member:${userId}`);
    const occupancy = Object.fromEntries(
      Object.entries(this.#snapshot.occupancy).map(([channelId, occupants]) => [
        channelId,
        occupants.filter((occupant) => occupant.userId !== userId),
      ]),
    );
    this.#update({
      members: this.#snapshot.members.filter((member) => member.id !== userId),
      occupancy,
    });
  }

  setPermissions(permissions: ServerPermission[]): void {
    this.#touch("permissions");
    this.#update({ permissions: [...permissions] });
  }

  setServer(server: Server): void {
    this.#touch("server", "servers");
    this.#update({
      server,
      servers: this.#snapshot.servers.map((item) =>
        item.id === server.id ? server : item,
      ),
    });
  }

  setServerIcon(iconUrl: string | undefined): void {
    const current = this.#snapshot.server;
    if (current === null) return;
    this.#touch("server");
    const { iconUrl: _previous, ...rest } = current;
    this.#update({ server: iconUrl === undefined ? rest : { ...rest, iconUrl } });
  }

  /**
   * The hub sends a room's whole list on every change in it, mute and display
   * name included; a list that says what the directory already shows
   * publishes nothing.
   */
  setOccupants(channelId: ChannelId, occupants: VoiceOccupant[]): void {
    this.#touch(`room:${channelId}`);
    if (sameOccupants(this.#snapshot.occupancy[channelId] ?? [], occupants)) return;
    this.#update({ occupancy: { ...this.#snapshot.occupancy, [channelId]: occupants } });
  }

  addChannel(channel: Channel): void {
    this.#touch("channels");
    this.#update({ channels: [...this.#snapshot.channels, channel].sort(byPosition) });
  }

  setChannelSettings(channelId: ChannelId, patch: Partial<Channel>): void {
    this.#touch("channels");
    this.#update({
      channels: this.#snapshot.channels.map((channel) =>
        channel.id === channelId ? { ...channel, ...patch } : channel,
      ),
    });
  }

  /**
   * The order of the channels and the section of each, in one publish, so
   * the rail never shows a channel moved but not yet placed. Channels missing
   * from `placements` keep their section and relative order, after the rest.
   * An unchanged layout publishes nothing: the hub echoes every reorder back
   * to the client that made it, and a fresh snapshot would end a drag that
   * began since.
   */
  setChannelLayout(placements: readonly ChannelPlacement[]): void {
    this.#touch("channels");
    const sections = new Map(placements.map(({ id, sectionId }) => [id, sectionId]));
    const placed = this.#snapshot.channels.map((channel) => {
      const sectionId = sections.get(channel.id);
      if (sectionId === undefined || sectionId === (channel.sectionId ?? null)) return channel;
      return sectionId === null ? withoutSection(channel) : { ...channel, sectionId };
    });
    const channels = inOrder(
      placed,
      placements.map(({ id }) => id),
    );
    const current = this.#snapshot.channels;
    if (channels.every((channel, index) => channel === current[index])) return;
    this.#update({ channels });
  }

  removeChannel(channelId: ChannelId): void {
    this.#touch("channels", `room:${channelId}`);
    const { [channelId]: _closed, ...occupancy } = this.#snapshot.occupancy;
    this.#update({
      channels: this.#snapshot.channels.filter((channel) => channel.id !== channelId),
      occupancy,
    });
  }

  addSection(section: Section): void {
    this.#touch("sections");
    this.#update({ sections: [...this.#snapshot.sections, section].sort(byOrder) });
  }

  setSection(section: Section): void {
    this.#touch("sections");
    this.#update({
      sections: this.#snapshot.sections.map((item) =>
        item.id === section.id ? section : item,
      ),
    });
  }

  /** Like the channel layout, an unchanged order publishes nothing. */
  setSectionOrder(orderedIds: readonly string[]): void {
    this.#touch("sections");
    const sections = inOrder(this.#snapshot.sections, orderedIds);
    const current = this.#snapshot.sections;
    if (sections.every((section, index) => section === current[index])) return;
    this.#update({ sections });
  }

  removeSection(sectionId: string): void {
    this.#touch("sections", "channels");
    this.#update({
      sections: this.#snapshot.sections.filter((section) => section.id !== sectionId),
      // The server refuses to delete a section that channels still use, but a
      // channel unassigned a moment earlier must not keep a dangling id.
      channels: this.#snapshot.channels.map((channel) =>
        channel.sectionId === sectionId ? withoutSection(channel) : channel,
      ),
    });
  }

  #touch(...parts: Part[]): void {
    this.#changes += 1;
    for (const part of parts) this.#touched.set(part, this.#changes);
  }

  #replaceMembers(replace: (member: User) => User): void {
    this.#update({ members: this.#snapshot.members.map(replace) });
  }

  #update(patch: Partial<DirectorySnapshot>): void {
    this.#publish({ ...this.#snapshot, ...patch });
  }

  #publish(next: DirectorySnapshot): void {
    this.#snapshot = next;
    for (const listener of this.#listeners) listener();
  }
}

/** `previous` when `next` says the same, compared as the plain data both are. */
function kept<T>(previous: T, next: T): T {
  return samePlain(previous, next) ? previous : next;
}

/** Equal as JSON data, whatever order the fields came in; an absent field and `undefined` are one. */
function samePlain(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (typeof a !== "object" || typeof b !== "object" || a === null || b === null) return false;
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  const fields = (value: object) =>
    Object.entries(value).filter(([, field]) => field !== undefined);
  const left = fields(a);
  const right = new Map(fields(b));
  return left.length === right.size && left.every(([key, field]) => samePlain(field, right.get(key)));
}

/** Each item kept as it was where it says the same, and the list itself when every item is. */
function keptList<T extends string | { id: string }>(previous: T[], next: readonly T[]): T[] {
  const key = (item: T) => (typeof item === "string" ? item : item.id);
  const before = new Map(previous.map((item) => [key(item), item]));
  const items = next.map((item) => {
    const old = before.get(key(item));
    return old === undefined ? item : kept(old, item);
  });
  return items.length === previous.length && items.every((item, index) => item === previous[index])
    ? previous
    : items;
}

function sameOccupants(a: readonly VoiceOccupant[], b: readonly VoiceOccupant[]): boolean {
  return (
    a.length === b.length &&
    a.every((occupant, index) => {
      const other = b[index];
      return (
        other !== undefined &&
        occupant.userId === other.userId &&
        occupant.displayName === other.displayName &&
        occupant.muted === other.muted
      );
    })
  );
}

function byPosition(a: Channel, b: Channel): number {
  return (a.position ?? 0) - (b.position ?? 0) || a.name.localeCompare(b.name);
}

function byOrder(a: Section, b: Section): number {
  return a.position - b.position || a.name.localeCompare(b.name);
}

function inOrder<T extends { id: string }>(items: T[], orderedIds: readonly string[]): T[] {
  const rank = new Map(orderedIds.map((id, position) => [id, position]));
  const rankOf = ({ id }: T): number => rank.get(id) ?? orderedIds.length;
  return [...items].sort((a, b) => rankOf(a) - rankOf(b));
}

function withoutSection({ sectionId: _sectionId, ...channel }: Channel): Channel {
  return channel;
}
