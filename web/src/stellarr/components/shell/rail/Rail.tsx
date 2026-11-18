import { useId, useState, type MouseEvent, type ReactNode } from "react";

import { Icon } from "@stellarr/icons";
import { useContextMenu } from "@/components/primitives/ContextMenu";
import { UserMenu, type UserMenuActions } from "@/components/roster/UserMenu";
import {
  ChannelMenu,
  type ChannelMenuActions,
} from "@/components/shell/rail/ChannelMenu";
import { CollapsibleGroup } from "@/components/shell/rail/CollapsibleGroup";
import { RailHeader } from "@/components/shell/rail/RailHeader";
import { RailSection } from "@/components/shell/rail/RailSection";
import { RailTextList } from "@/components/shell/rail/RailTextList";
import { RailVoiceList } from "@/components/shell/rail/RailVoiceList";
import type {
  ChannelMenuRaiser,
  ChannelMenuTarget,
  RailSortRefs,
  RailThreads,
} from "@/components/shell/rail/railTypes";
import { useThreadFocus } from "@/components/shell/rail/useThreadFocus";
import { useVoiceMove } from "@/components/shell/rail/useVoiceMove";
import type {
  ChannelMove,
  ChannelThreadVM,
  PersonVM,
  SectionVM,
  TextChannelVM,
  VoiceChannelVM,
} from "@/components/view-models";
import { LiveDragAnnouncer } from "@/lib/drag/DragAnnouncer";
import { useListSort, type SortList, type SortPlace } from "@/lib/drag/useListSort";
import { useLanguage } from "@/state/hooks";

export { voiceMoveRequest } from "@/components/shell/rail/voiceMove";

/** One group's channels of one kind, as the rail's sort sees them. */
interface RailList extends SortList {
  sectionId: string | null;
  /** What the group is called when a screen reader says where a channel is. */
  name: string;
  /** The id of the collapsible body the list is in. */
  body: string;
}

const listId = (kind: "text" | "voice", sectionId: string | null) =>
  `${kind}:${sectionId ?? ""}`;

export interface RailProps {
  serverName: string;
  textChannels: TextChannelVM[];
  /** Complete thread lists keyed by their text channel. */
  threadsByChannel?: Readonly<Record<string, ChannelThreadVM[]>>;
  activeThreadId?: string | null;
  openingThreadId?: string | null;
  onOpenThread?: (thread: ChannelThreadVM) => void;
  voiceChannels: VoiceChannelVM[];
  /** Collapsible rail groups, in display order. Empty until any exist. */
  sections?: SectionVM[];
  /** The channel the room is showing — text or voice, only one is current. */
  activeChannelId: string | null;
  /** Whose occupant row gets `.me`. */
  selfId: string;
  /**
   * Whether this person may edit the channels: it decides the cog on each row
   * and the "+" beside each caption. The rail renders what it is told; who is
   * allowed is a question for whoever knows about roles.
   */
  canManageChannels?: boolean;
  /** Whether at least one server-settings pane is available to this member. */
  canAccessServerSettings?: boolean;
  /** Invites and server-wide administration require ManageServer. */
  canManageServer?: boolean;
  onSelectTextChannel: (id: string) => void;
  onJoinVoiceChannel: (id: string) => void;
  onEditServer: () => void;
  onInvite: () => void;
  /** A channel dropped in the rail; every channel outside its group keeps its position. */
  onMoveChannel: (move: ChannelMove) => Promise<void> | void;
  onOpenChannelSettings?: (id: string) => void;
  onCreateChannel?: (kind: "text" | "voice") => void;
  /**
   * A section's cog. Sections are created from server settings, not the rail:
   * the "+" beside a caption makes a channel, and a second "+" for a whole
   * rail group would read as one control too many for something used far less
   * often.
   */
  onOpenSectionSettings?: (sectionId: string) => void;
  /**
   * Right-click on a channel row. Absent leaves the browser's own menu alone;
   * what the menu offers is decided by which of its actions are given.
   */
  channelMenu?: ChannelMenuActions;
  /** Right-click actions for a person seated under a voice channel. */
  voiceMemberMenu?: UserMenuActions;
  /** Staff-only move; dropping on the source channel is ignored. */
  onMoveVoiceMember?: (
    userId: string,
    sourceChannelId: string,
    targetChannelId: string,
  ) => Promise<void> | void;
  /** `<VoiceDock>`, composed by the container. */
  voiceDock?: ReactNode;
  /** `<AccountRow>`, composed by the container. */
  account?: ReactNode;
  /**
   * Below 768 the rail is an off-screen drawer while shut. Off screen must
   * also mean out of the tab order and off the accessibility tree, the same
   * way the roster handles it.
   */
  hidden?: boolean;
}

/** Whichever menu is open: one for the whole rail, never one per row. */
type RailMenu =
  | { type: "channel"; channel: ChannelMenuTarget }
  | { type: "person"; person: PersonVM };

function toggled<T>(set: ReadonlySet<T>, value: T): Set<T> {
  const next = new Set(set);
  if (next.has(value)) next.delete(value);
  else next.add(value);
  return next;
}

function CreateChannelButton({
  label,
  onCreate,
}: {
  label: string;
  onCreate: () => void;
}) {
  const { t } = useLanguage();
  return (
    <button
      type="button"
      className="rail-add"
      title={t("shell.createChannel")}
      aria-label={label}
      onClick={onCreate}
    >
      <Icon name="plus" size={14} />
    </button>
  );
}

export function Rail({
  serverName,
  textChannels,
  threadsByChannel = {},
  activeThreadId = null,
  openingThreadId = null,
  onOpenThread,
  voiceChannels,
  sections = [],
  activeChannelId,
  selfId,
  canManageChannels = false,
  canAccessServerSettings = false,
  canManageServer = false,
  onSelectTextChannel,
  onJoinVoiceChannel,
  onEditServer,
  onInvite,
  onMoveChannel,
  onOpenChannelSettings,
  onCreateChannel,
  onOpenSectionSettings,
  channelMenu,
  voiceMemberMenu,
  onMoveVoiceMember,
  voiceDock,
  account,
  hidden = false,
}: RailProps) {
  const { t } = useLanguage();
  const railId = useId();
  const [textOpen, setTextOpen] = useState(true);
  const [voiceOpen, setVoiceOpen] = useState(true);
  // Sections default open; tracking the closed ones means a freshly created
  // section needs no seeding to already look expanded.
  const [closedSections, setClosedSections] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const [expandedThreads, setExpandedThreads] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const [moveError, setMoveError] = useState<string | null>(null);
  const move = useVoiceMove({
    onMove: onMoveVoiceMember,
    onError: setMoveError,
  });

  /*
   * `useContextMenu` holds where the pointer was; which row it was over is
   * something it cannot know, so the target travels alongside it. Both are
   * facts about the event that raised the menu — reading the row back when an
   * item runs would give whatever the list looks like by then.
   */
  const ctx = useContextMenu();
  const [menu, setMenu] = useState<RailMenu | null>(null);

  const raiseChannelMenu: ChannelMenuRaiser | undefined =
    channelMenu === undefined
      ? undefined
      : (channel) => (event: MouseEvent<HTMLElement>) => {
          setMenu({ type: "channel", channel });
          ctx.onContextMenu(event);
        };
  const raisePersonMenu =
    voiceMemberMenu === undefined
      ? undefined
      : (person: PersonVM) => (event: MouseEvent<HTMLElement>) => {
          setMenu({ type: "person", person });
          ctx.onContextMenu(event);
        };

  const moveChannel = async (move: ChannelMove) => {
    setMoveError(null);
    try {
      await onMoveChannel(move);
    } catch {
      setMoveError(t("rail.reorderFailed"));
    }
  };

  // Every group of the rail is a list of one sort, so a channel can be
  // dragged into any group of its kind. Sorting is for whoever may edit the
  // channels; anyone else gets plain rows.
  const canSort = canManageChannels;
  const looseText = textChannels.filter((channel) => channel.sectionId === undefined);
  const looseVoice = voiceChannels.filter((channel) => channel.sectionId === undefined);
  const grouped = sections.map((section) => ({
    section,
    text: textChannels.filter((channel) => channel.sectionId === section.id),
    voice: voiceChannels.filter((channel) => channel.sectionId === section.id),
  }));
  const ids = (channels: { id: string }[]) => channels.map((channel) => channel.id);
  const lists: RailList[] = [
    {
      id: listId("text", null),
      kind: "text",
      items: ids(looseText),
      closed: !textOpen,
      sectionId: null,
      name: t("shell.textChannels"),
      body: `${railId}-text`,
    },
    {
      id: listId("voice", null),
      kind: "voice",
      items: ids(looseVoice),
      closed: !voiceOpen,
      sectionId: null,
      name: t("shell.voiceChannels"),
      body: `${railId}-voice`,
    },
    ...grouped.flatMap(({ section, text, voice }) => {
      const group = {
        closed: closedSections.has(section.id),
        sectionId: section.id,
        name: section.name,
        body: `${railId}-section-${section.id}`,
      };
      return [
        { ...group, id: listId("text", section.id), kind: "text", items: ids(text) },
        { ...group, id: listId("voice", section.id), kind: "voice", items: ids(voice) },
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
  const {
    container: sortContainer,
    list: sortList,
    item: sortItem,
    handle: sortHandle,
    announcer: sortAnnouncer,
    instructionsId: sortInstructionsId,
  } = useListSort({
    lists,
    onMove: ({ id, list, order }) =>
      void moveChannel({ channelId: id, sectionId: listOf(list)?.sectionId ?? null, order }),
    roleDescription: t("shell.reorderHandle"),
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
  });
  const sortRefs = (list: string): RailSortRefs => ({
    list: sortList(list),
    item: sortItem,
    ...(canSort ? { handle: sortHandle } : {}),
  });

  const registerThread = useThreadFocus(threadsByChannel, activeThreadId, hidden);
  const threads: RailThreads = {
    byChannel: threadsByChannel,
    activeId: activeThreadId,
    openingId: openingThreadId,
    expanded: expandedThreads,
    onToggle: (channelId) =>
      setExpandedThreads((current) => toggled(current, channelId)),
    onOpen: onOpenThread,
    register: registerThread,
  };
  const openSettings = canManageChannels ? onOpenChannelSettings : undefined;

  const textList = (channels: TextChannelVM[], sectionId: string | null) => (
    <RailTextList
      channels={channels}
      activeChannelId={activeChannelId}
      threads={threads}
      isMuted={channelMenu?.isMuted}
      onSelect={onSelectTextChannel}
      onOpenSettings={openSettings}
      menuFor={raiseChannelMenu}
      sort={sortRefs(listId("text", sectionId))}
    />
  );
  const voiceList = (channels: VoiceChannelVM[], sectionId: string | null) => (
    <RailVoiceList
      channels={channels}
      activeChannelId={activeChannelId}
      selfId={selfId}
      move={move}
      canMove={onMoveVoiceMember !== undefined}
      onJoin={onJoinVoiceChannel}
      onOpenSettings={openSettings}
      menuFor={raiseChannelMenu}
      personMenuFor={raisePersonMenu}
      sort={sortRefs(listId("voice", sectionId))}
    />
  );

  const createAction = (kind: "text" | "voice") =>
    canManageChannels && onCreateChannel !== undefined ? (
      <CreateChannelButton
        label={t(
          kind === "text"
            ? "shell.createTextChannel"
            : "shell.createVoiceChannel",
        )}
        onCreate={() => onCreateChannel(kind)}
      />
    ) : null;

  let openMenu: ReactNode = null;
  if (ctx.at !== null && menu?.type === "channel" && channelMenu !== undefined) {
    openMenu = (
      <ChannelMenu
        {...channelMenu}
        at={ctx.at}
        channelId={menu.channel.id}
        kind={menu.channel.kind}
        name={menu.channel.name}
        unread={menu.channel.unread}
        currentSectionId={menu.channel.sectionId}
        onClose={ctx.close}
      />
    );
  } else if (
    ctx.at !== null &&
    menu?.type === "person" &&
    voiceMemberMenu !== undefined
  ) {
    openMenu = (
      <UserMenu
        {...voiceMemberMenu}
        at={ctx.at}
        userId={menu.person.id}
        displayName={menu.person.displayName}
        self={menu.person.id === selfId}
        anchor={ctx.anchor}
        onClose={ctx.close}
      />
    );
  }

  return (
    <nav
      className="rail"
      aria-label={t("shell.serverAndChannels")}
      // `inert` already removes a closed drawer from focus navigation and the
      // accessibility tree. Adding `aria-hidden` as well races with focus
      // transfer when a channel click closes the drawer, which Chromium
      // correctly reports as hiding the currently focused control.
      inert={hidden}
    >
      <RailHeader
        serverName={serverName}
        onEditServer={canAccessServerSettings ? onEditServer : undefined}
        onInvite={canManageServer ? onInvite : undefined}
      />

      <div className="rail-scroll" ref={sortContainer}>
        <CollapsibleGroup
          id={`${railId}-text`}
          caption={t("shell.textChannels")}
          captionTitle={canSort ? t("shell.dragChannel") : undefined}
          open={textOpen}
          onToggle={() => setTextOpen((open) => !open)}
          actions={createAction("text")}
        >
          {textList(looseText, null)}
        </CollapsibleGroup>
        <CollapsibleGroup
          id={`${railId}-voice`}
          caption={t("shell.voiceChannels")}
          open={voiceOpen}
          onToggle={() => setVoiceOpen((open) => !open)}
          actions={createAction("voice")}
        >
          {voiceList(looseVoice, null)}
        </CollapsibleGroup>

        {grouped.map(({ section, text, voice }) => (
          <RailSection
            key={section.id}
            id={`${railId}-section-${section.id}`}
            name={section.name}
            open={!closedSections.has(section.id)}
            onToggle={() => setClosedSections((current) => toggled(current, section.id))}
            empty={text.length === 0 && voice.length === 0}
            onOpenSettings={
              canManageChannels && onOpenSectionSettings !== undefined
                ? () => onOpenSectionSettings(section.id)
                : undefined
            }
          >
            {textList(text, section.id)}
            {voiceList(voice, section.id)}
          </RailSection>
        ))}

        {moveError === null ? null : (
          <p className="rail-move-error" role="alert">
            {moveError}
          </p>
        )}
      </div>

      <LiveDragAnnouncer
        id={sortInstructionsId}
        instructions={t("shell.reorderInstructions")}
        announcer={sortAnnouncer}
      />
      <span className="sr-only" aria-live="polite">
        {move.announcement}
      </span>

      {/* Rendered from the rail rather than from the row, so the menu outlives
          a list that reorders under it and there is only ever one open. */}
      {openMenu}

      {voiceDock}
      {account}
    </nav>
  );
}
