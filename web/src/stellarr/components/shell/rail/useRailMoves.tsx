import { useId, useSyncExternalStore, type RefCallback } from "react";
import { createPortal } from "react-dom";

import { Avatar } from "@/components/primitives/Avatar";
import type { RailSortRefs } from "@/components/shell/rail/railTypes";
import type { VoiceDragData } from "@/components/shell/rail/voiceMove";
import type {
  ChannelMove,
  PersonVM,
  SectionVM,
  TextChannelVM,
  VoiceChannelVM,
} from "@/components/view-models";
import { DragAnnouncer } from "@/lib/drag/DragAnnouncer";
import {
  useRailDrag,
  type DragHandleProps,
  type SortList,
  type SortPlace,
} from "@/lib/drag/useRailDrag";
import { useLanguage } from "@/state/hooks";

/** One group's channels of one kind, as the rail's sort sees them. */
interface RailList extends SortList {
  sectionId: string | null;
  /** What the group is called when a screen reader says where a channel is. */
  name: string;
  /** The id of the collapsible body the list is in. */
  body: string;
}

export const listId = (kind: "text" | "voice", sectionId: string | null) =>
  `${kind}:${sectionId ?? ""}`;

/** One seated person's carry key: the same person can only sit in one channel at a time. */
const occupantKey = (channelId: string, personId: string) => `${channelId}:${personId}`;

export interface RailMovesOptions {
  /** The rail's own id, which the collapsible bodies' ids start with. */
  railId: string;
  textChannels: TextChannelVM[];
  voiceChannels: VoiceChannelVM[];
  sections: SectionVM[];
  textOpen: boolean;
  voiceOpen: boolean;
  closedSections: ReadonlySet<string>;
  /** Whether channels can be sorted at all; without it the rows are plain buttons. */
  canSortChannels: boolean;
  onMoveChannel: (move: ChannelMove) => void;
  /** Whose own row is never carried. */
  selfId: string;
  /** Whose move the server has not confirmed yet, and who is not carried meanwhile. */
  movingUserId: string | null;
  /** Absent leaves people as plain rows. */
  onDropPerson?: (dragged: VoiceDragData, targetChannelId: string | null) => void;
}

/**
 * The live region of every drag, and the copy of a person being carried,
 * which is rendered over the page while someone is carried and placed by the
 * drag itself.
 */
function DragLayer({
  announcer,
  instructions,
  carrying,
  personOf,
  copy,
}: {
  announcer: ReturnType<typeof useRailDrag>["announcer"];
  instructions: { id: string; text: string }[];
  carrying: ReturnType<typeof useRailDrag>["carrying"];
  personOf: (key: string) => PersonVM | undefined;
  copy: RefCallback<HTMLElement>;
}) {
  const key = useSyncExternalStore(carrying.subscribe, carrying.read, carrying.read);
  const person = key === null ? undefined : personOf(key);
  return (
    <>
      <DragAnnouncer instructions={instructions} announcer={announcer} />
      {person === undefined || typeof document === "undefined"
        ? null
        : createPortal(
            <div className="voice-occupant-carry" ref={copy}>
              <div className="voice-occupant-overlay">
                <Avatar
                  id={person.id}
                  displayName={person.displayName}
                  initials={person.initials}
                  avatarUrl={person.avatarUrl}
                  size="xs"
                />
                <span>{person.displayName}</span>
              </div>
            </div>,
            document.body,
          )}
    </>
  );
}

/**
 * Every drag of the rail, set up over its groups: channels sorted across
 * every group of their kind, and people carried onto any voice channel.
 */
export function useRailMoves({
  railId,
  textChannels,
  voiceChannels,
  sections,
  textOpen,
  voiceOpen,
  closedSections,
  canSortChannels,
  onMoveChannel,
  selfId,
  movingUserId,
  onDropPerson,
}: RailMovesOptions) {
  const { t } = useLanguage();
  const channelInstructions = useId();
  const personInstructions = useId();

  const ids = (channels: { id: string }[]) => channels.map((channel) => channel.id);
  const loose = (channel: { sectionId?: string }) => channel.sectionId === undefined;
  const lists: RailList[] = [
    {
      id: listId("text", null),
      kind: "text",
      items: ids(textChannels.filter(loose)),
      closed: !textOpen,
      sectionId: null,
      name: t("shell.textChannels"),
      body: `${railId}-text`,
    },
    {
      id: listId("voice", null),
      kind: "voice",
      items: ids(voiceChannels.filter(loose)),
      closed: !voiceOpen,
      sectionId: null,
      name: t("shell.voiceChannels"),
      body: `${railId}-voice`,
    },
    ...sections.flatMap((section) => {
      const group = {
        closed: closedSections.has(section.id),
        sectionId: section.id,
        name: section.name,
        body: `${railId}-section-${section.id}`,
      };
      const inSection = (channel: { sectionId?: string }) => channel.sectionId === section.id;
      return [
        { ...group, id: listId("text", section.id), kind: "text", items: ids(textChannels.filter(inSection)) },
        { ...group, id: listId("voice", section.id), kind: "voice", items: ids(voiceChannels.filter(inSection)) },
      ];
    }),
  ];
  const listOf = (id: string) => lists.find((list) => list.id === id);
  const channelName = (id: string) =>
    [...textChannels, ...voiceChannels].find((channel) => channel.id === id)?.name ?? "";
  const placeWords = (id: string, { list, position, count }: SortPlace) => ({
    name: channelName(id),
    position,
    count,
    group: listOf(list)?.name ?? "",
  });

  // Who can be carried now: never you, and never someone already on the way.
  const carriable = new Map<string, VoiceDragData>();
  if (onDropPerson !== undefined) {
    for (const channel of voiceChannels) {
      for (const person of channel.occupants) {
        if (person.id === selfId || person.id === movingUserId) continue;
        carriable.set(occupantKey(channel.id, person.id), {
          type: "voice-occupant",
          person,
          sourceChannelId: channel.id,
        });
      }
    }
  }

  const { container, list, item, handle, carry, copy, announcer, carrying } = useRailDrag({
    lists,
    onMove: ({ id, list, order }) =>
      onMoveChannel({ channelId: id, sectionId: listOf(list)?.sectionId ?? null, order }),
    messages: {
      lifted: (id, place) => t("shell.reorderLifted", placeWords(id, place)),
      moved: (id, place) => t("shell.reorderMoved", placeWords(id, place)),
      dropped: (id, place) => t("shell.reorderDropped", placeWords(id, place)),
      cancelled: () => t("shell.reorderCancelled"),
    },
    // The rows of a closed group cannot take focus; the caption that opens it can.
    focusClosed: (list) => {
      const body = listOf(list)?.body;
      return (
        [...document.querySelectorAll<HTMLElement>("[aria-controls]")].find(
          (caption) => caption.getAttribute("aria-controls") === body,
        ) ?? null
      );
    },
    ...(onDropPerson === undefined
      ? {}
      : {
          drops: {
            zoneKind: "voice",
            carriable: [...carriable.keys()],
            homeOf: (key) => carriable.get(key)?.sourceChannelId ?? "",
            messages: {
              lifted: (key) =>
                t("shell.dragLifted", { name: carriable.get(key)?.person.displayName ?? "" }),
              over: (channelId) => t("shell.dragOver", { name: channelName(channelId) }),
              dropped: (key, channelId) =>
                channelId === null
                  ? t("shell.dragCancelled")
                  : t("shell.dragDropped", {
                      user: carriable.get(key)?.person.displayName ?? "",
                      channel: channelName(channelId),
                    }),
              cancelled: () => t("shell.dragCancelled"),
            },
            commit: (key, channelId) => {
              const dragged = carriable.get(key);
              if (dragged !== undefined) onDropPerson(dragged, channelId);
            },
          },
        }),
  });

  const channelHandle = { roleDescription: t("shell.reorderHandle"), instructionsId: channelInstructions };
  const personHandle = { roleDescription: t("shell.dragHandle"), instructionsId: personInstructions };

  return {
    container,
    /** What the list of one kind in one group takes from the drags. */
    refsFor: (kind: "text" | "voice", sectionId: string | null): RailSortRefs => ({
      list: list(listId(kind, sectionId)),
      item,
      ...(canSortChannels ? { handle: (id: string) => handle(id, channelHandle) } : {}),
    }),
    /** The handle that carries a seated person, for whoever can be carried. */
    carry: (channelId: string, person: PersonVM): DragHandleProps | undefined => {
      const key = occupantKey(channelId, person.id);
      return carriable.has(key) ? carry(key, personHandle) : undefined;
    },
    layer: (
      <DragLayer
        announcer={announcer}
        instructions={[
          { id: channelInstructions, text: t("shell.reorderInstructions") },
          { id: personInstructions, text: t("shell.dragInstructions") },
        ]}
        carrying={carrying}
        personOf={(key) => carriable.get(key)?.person}
        copy={copy}
      />
    ),
  };
}
