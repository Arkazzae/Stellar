import { rememberActiveServerId } from "@/lib/api/transport";
import type { Channel, ChannelPlacement, ChannelSectionMove, Server } from "@/lib/api/types";
import type { DirectoryPort } from "@/ports";

import type { DirectoryStore } from "./store";

/** Shown until the bootstrap snapshot arrives. */
const UNKNOWN_SERVER: Server = { id: "", name: "Stellarr" };

type PersistOrder = (orderedIds: string[]) => Promise<void>;
type PersistChannelOrder = (orderedIds: string[], move?: ChannelSectionMove) => Promise<void>;

const placementsOf = (channels: readonly Channel[]): ChannelPlacement[] =>
  channels.map(({ id, sectionId }) => ({ id, sectionId: sectionId ?? null }));

/**
 * `DirectoryPort` over the shared directory store.
 *
 * Reordering is optimistic: the store changes at once so a drag does not snap
 * back, and the previous order, with every channel's section, returns if the
 * server refuses.
 */
export function createDirectoryAdapter(
  store: DirectoryStore,
  persistOrder?: PersistChannelOrder,
  persistSectionOrder?: PersistOrder,
  serverActions?: {
    create: DirectoryPort["createServer"];
    discover: DirectoryPort["discoverServers"];
    join: DirectoryPort["joinServer"];
    joinInvite: DirectoryPort["joinInvite"];
  },
): DirectoryPort {
  return {
    getServer: () => store.getSnapshot().server ?? UNKNOWN_SERVER,
    listServers: () => store.getSnapshot().servers,
    listChannels: () => store.getSnapshot().channels,
    listSections: () => store.getSnapshot().sections,
    listPermissions: () => store.getSnapshot().permissions,
    onChanged: (handler) => store.subscribe(handler),

    reorderChannels: (orderedIds, move) => {
      const before = placementsOf(store.getSnapshot().channels);
      const sectionOf = new Map(before.map(({ id, sectionId }) => [id, sectionId]));
      const after = orderedIds.map((id) => ({
        id,
        sectionId: id === move?.channelId ? move.sectionId : (sectionOf.get(id) ?? null),
      }));
      return persistOptimistically(
        () => store.setChannelLayout(after),
        () => store.setChannelLayout(before),
        persistOrder && (() => persistOrder(orderedIds, move)),
      );
    },

    reorderSections: (orderedIds) => {
      const before = store.getSnapshot().sections.map((section) => section.id);
      return persistOptimistically(
        () => store.setSectionOrder(orderedIds),
        () => store.setSectionOrder(before),
        persistSectionOrder && (() => persistSectionOrder(orderedIds)),
      );
    },

    async createServer(input) {
      if (serverActions === undefined) throw new Error("Server creation is unavailable.");
      return serverActions.create(input);
    },

    async discoverServers() {
      return serverActions?.discover() ?? [];
    },

    async joinServer(serverId) {
      if (serverActions === undefined) throw new Error("Joining servers is unavailable.");
      return serverActions.join(serverId);
    },

    async joinInvite(code) {
      if (serverActions === undefined) throw new Error("Joining by invite is unavailable.");
      return serverActions.joinInvite(code);
    },

    selectServer(serverId) {
      rememberActiveServerId(serverId);
    },
  };
}

async function persistOptimistically(
  apply: () => void,
  restore: () => void,
  persist: (() => Promise<void>) | undefined,
): Promise<void> {
  apply();
  if (persist === undefined) return;
  try {
    await persist();
  } catch (error) {
    restore();
    throw error;
  }
}
