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
import type { MessageKey } from "@/i18n";
import { useLanguage } from "@/state/hooks";

/** One group's channels of one kind, or the sections themselves, as the rail's sort sees them. */
interface RailList extends SortList {
  sectionId: string | null;
  /** What the group is called when a screen reader says where a channel is. */
  name: string;
  /** The id of the collapsible body the list is in. */
  body: string;
}

export const listId = (kind: "text" | "voice", sectionId: string | null) =>
  `${kind}:${sectionId ?? ""}`;

/** The sections are one more list, below the loose groups, which stay on top. */
const SECTIONS = "sections";

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
  /** Whose own row carries them to the channel they drop it on, by joining it. */
  selfId: string;
  /** Joins a voice channel, as a click on it does. */
  onJoin: (channelId: string) => void;
  /** Whose move the server has not confirmed yet, and who is not carried meanwhile. */
  movingUserId: string | null;
  /** Absent leaves everyone but you as plain rows. */
  onDropPerson?: (dragged: VoiceDragData, targetChannelId: string | null) => void;
  /** Absent leaves the section captions as plain buttons. */
  onMoveSection?: (order: string[]) => void;
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
 * every group of their kind, sections sorted by their captions, and people
 * carried onto any voice channel: others by staff, and you by yourself onto a
 * channel you can join.
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
  onJoin,
  movingUserId,
  onDropPerson,
  onMoveSection,
}: RailMovesOptions) {
  const { t } = useLanguage();
  const channelInstructions = useId();
  const sectionInstructions = useId();
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
    {
      id: SECTIONS,
      kind: "section",
      items: ids(sections),
      closed: false,
      sectionId: null,
      name: "",
      body: "",
    },
  ];
  const listOf = (id: string) => lists.find((list) => list.id === id);
  const channelName = (id: string) =>
    [...textChannels, ...voiceChannels].find((channel) => channel.id === id)?.name ?? "";
  const sectionName = (id: string) => sections.find((section) => section.id === id)?.name;
  const placeWords = (id: string, { list, position, count }: SortPlace) => ({
    name: sectionName(id) ?? channelName(id),
    position,
    count,
    group: listOf(list)?.name ?? "",
  });
  /** The channel sentence, or the section one when a section is what is carried. */
  const said = (channel: MessageKey, section: MessageKey) => (id: string, place: SortPlace) =>
    t(place.list === SECTIONS ? section : channel, placeWords(id, place));

  // Who can be carried now: you, wherever you sit, and for staff everyone
  // else not already on the way.
  const carriable = new Map<string, VoiceDragData>();
  for (const channel of voiceChannels) {
    for (const person of channel.occupants) {
      const you = person.id === selfId;
      if (!you && (onDropPerson === undefined || person.id === movingUserId)) continue;
      carriable.set(occupantKey(channel.id, person.id), {
        type: "voice-occupant",
        person,
        sourceChannelId: channel.id,
      });
    }
  }
  const isSelf = (key: string) => carriable.get(key)?.person.id === selfId;
  // A full channel turns newcomers away, as the server does on a click.
  const joinable = (id: string) => {
    const channel = voiceChannels.find((each) => each.id === id);
    return (
      channel !== undefined &&
      (channel.userLimit === undefined || channel.occupants.length < channel.userLimit)
    );
  };

  const { container, list, item, handle, carry, copy, announcer, carrying } = useRailDrag({
    lists,
    onMove: ({ id, list, order }) => {
      if (list === SECTIONS) onMoveSection?.(order);
      else onMoveChannel({ channelId: id, sectionId: listOf(list)?.sectionId ?? null, order });
    },
    messages: {
      lifted: said("shell.reorderLifted", "shell.sectionLifted"),
      moved: said("shell.reorderMoved", "shell.sectionMoved"),
      dropped: said("shell.reorderDropped", "shell.sectionDropped"),
      cancelled: (id) =>
        t(sectionName(id) === undefined ? "shell.reorderCancelled" : "shell.sectionCancelled"),
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
    drops: {
      zoneKind: "voice",
      carriable: [...carriable.keys()],
      homeOf: (key) => carriable.get(key)?.sourceChannelId ?? "",
      // Staff move people wherever they like; you go only where a click would take you.
      accepts: (key, channelId) => !isSelf(key) || joinable(channelId),
      messages: {
        lifted: (key) =>
          isSelf(key)
            ? t("shell.dragSelfLifted")
            : t("shell.dragLifted", { name: carriable.get(key)?.person.displayName ?? "" }),
        over: (channelId) => t("shell.dragOver", { name: channelName(channelId) }),
        dropped: (key, channelId) => {
          if (channelId === null) return t("shell.dragCancelled");
          if (!isSelf(key)) {
            return t("shell.dragDropped", {
              user: carriable.get(key)?.person.displayName ?? "",
              channel: channelName(channelId),
            });
          }
          return channelId === carriable.get(key)?.sourceChannelId
            ? t("shell.dragCancelled")
            : t("shell.dragSelfDropped", { channel: channelName(channelId) });
        },
        cancelled: () => t("shell.dragCancelled"),
      },
      commit: (key, channelId) => {
        const dragged = carriable.get(key);
        if (dragged === undefined) return;
        // You switch channels the way a click does, never through the staff move.
        if (isSelf(key)) {
          if (channelId !== null && channelId !== dragged.sourceChannelId) onJoin(channelId);
        } else {
          onDropPerson?.(dragged, channelId);
        }
      },
    },
  });

  const channelHandle = { roleDescription: t("shell.reorderHandle"), instructionsId: channelInstructions };
  const personHandle = { roleDescription: t("shell.dragHandle"), instructionsId: personInstructions };
  const sectionHandle = {
    roleDescription: t("shell.sectionHandle"),
    instructionsId: sectionInstructions,
  };

  return {
    container,
    /** What the list of one kind in one group takes from the drags. */
    refsFor: (kind: "text" | "voice", sectionId: string | null): RailSortRefs => ({
      list: list(listId(kind, sectionId)),
      item,
      ...(canSortChannels ? { handle: (id: string) => handle(id, channelHandle) } : {}),
    }),
    /** What the sections take from the drags: their list, each one, and its caption as the handle. */
    sectionRefs: {
      list: list(SECTIONS),
      item,
      ...(onMoveSection === undefined
        ? {}
        : { handle: (id: string) => handle(id, sectionHandle) }),
    },
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
          { id: sectionInstructions, text: t("shell.sectionInstructions") },
          { id: personInstructions, text: t("shell.dragInstructions") },
        ]}
        carrying={carrying}
        personOf={(key) => carriable.get(key)?.person}
        copy={copy}
      />
    ),
  };
}
