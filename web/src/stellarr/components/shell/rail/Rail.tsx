import {
  useCallback,
  useId,
  useMemo,
  useState,
  type MouseEvent,
  type ReactNode,
} from "react";

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
  RailThreads,
} from "@/components/shell/rail/railTypes";
import { useArrivals } from "@/components/shell/rail/useArrivals";
import { useRailMoves } from "@/components/shell/rail/useRailMoves";
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
import { useStableHandler } from "@/hooks/useStableHandler";
import type { MessageKey } from "@/i18n";
import { useLanguage } from "@/state/hooks";

export { voiceMoveRequest } from "@/components/shell/rail/voiceMove";

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
  /**
   * A section dragged by its caption: the complete new order of the
   * sections. Absent leaves the captions as plain buttons.
   */
  onMoveSection?: (order: string[]) => Promise<void> | void;
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
  onMoveSection,
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

  // Everything a row is handed is the same function for the rail's whole
  // life, so the rows can be memoised and render only when they changed.
  const raise = useStableHandler((target: RailMenu, event: MouseEvent<HTMLElement>) => {
    setMenu(target);
    ctx.onContextMenu(event);
  });
  const hasChannelMenu = channelMenu !== undefined;
  const hasPersonMenu = voiceMemberMenu !== undefined;
  const raiseChannelMenu = useMemo<ChannelMenuRaiser | undefined>(
    () =>
      hasChannelMenu
        ? (channel) => (event) => raise({ type: "channel", channel }, event)
        : undefined,
    [hasChannelMenu, raise],
  );
  const raisePersonMenu = useMemo(
    () =>
      hasPersonMenu
        ? (person: PersonVM) => (event: MouseEvent<HTMLElement>) =>
            raise({ type: "person", person }, event)
        : undefined,
    [hasPersonMenu, raise],
  );
  const selectText = useStableHandler(onSelectTextChannel);
  const joinVoice = useStableHandler(onJoinVoiceChannel);
  const openThread = useStableHandler(onOpenThread);
  const openSettings = useStableHandler(canManageChannels ? onOpenChannelSettings : undefined);
  const toggleThreads = useCallback(
    (channelId: string) => setExpandedThreads((current) => toggled(current, channelId)),
    [],
  );

  /** A change to the rail's layout, which says so when the server refuses it. */
  const rearrange = async (
    change: () => Promise<void> | void,
    failed: MessageKey = "rail.reorderFailed",
  ) => {
    setMoveError(null);
    try {
      await change();
    } catch {
      setMoveError(t(failed));
    }
  };

  // Sorting channels is for whoever may edit them, and moving other people
  // for staff; anyone else gets plain rows, but for their own seat.
  const canSort = canManageChannels;
  const looseText = textChannels.filter((channel) => channel.sectionId === undefined);
  const looseVoice = voiceChannels.filter((channel) => channel.sectionId === undefined);
  const showTextCaption = sections.length === 0 || looseText.length > 0;
  const showVoiceCaption = sections.length === 0 || looseVoice.length > 0;
  // Empty loose lists stay open as drop targets, even when their captions are hidden.
  const looseTextOpen = textOpen || !showTextCaption;
  const looseVoiceOpen = voiceOpen || !showVoiceCaption;
  const grouped = sections.map((section) => ({
    section,
    text: textChannels.filter((channel) => channel.sectionId === section.id),
    voice: voiceChannels.filter((channel) => channel.sectionId === section.id),
  }));
  const {
    container: dragContainer,
    refsFor,
    sectionRefs: { list: sectionsList, item: sectionItem, handle: sectionHandle },
    canMoveOthers,
    carry,
    layer: dragLayer,
  } = useRailMoves({
    railId,
    textChannels,
    voiceChannels,
    sections,
    textOpen: looseTextOpen,
    voiceOpen: looseVoiceOpen,
    closedSections,
    canSortChannels: canSort,
    onMoveChannel: (channelMove) => void rearrange(() => onMoveChannel(channelMove)),
    selfId,
    onJoin: onJoinVoiceChannel,
    movingUserId: move.movingUserId,
    ...(onMoveVoiceMember === undefined ? {} : { onDropPerson: move.onDrop }),
    ...(onMoveSection === undefined
      ? {}
      : {
          onMoveSection: (order: string[]) =>
            void rearrange(() => onMoveSection(order), "rail.sectionOrderFailed"),
        }),
  });

  const registerThread = useThreadFocus(threadsByChannel, activeThreadId, hidden);
  const arrives = useArrivals(voiceChannels);
  const threads: RailThreads = {
    byChannel: threadsByChannel,
    activeId: activeThreadId,
    openingId: openingThreadId,
    expanded: expandedThreads,
    onToggle: toggleThreads,
    onOpen: openThread,
    register: registerThread,
  };

  const textList = (channels: TextChannelVM[], sectionId: string | null) => (
    <RailTextList
      channels={channels}
      activeChannelId={activeChannelId}
      threads={threads}
      isMuted={channelMenu?.isMuted}
      onSelect={selectText}
      onOpenSettings={openSettings}
      menuFor={raiseChannelMenu}
      sort={refsFor("text", sectionId)}
    />
  );
  const voiceList = (channels: VoiceChannelVM[], sectionId: string | null) => (
    <RailVoiceList
      channels={channels}
      activeChannelId={activeChannelId}
      selfId={selfId}
      movingUserId={move.movingUserId}
      onJoin={joinVoice}
      onOpenSettings={openSettings}
      menuFor={raiseChannelMenu}
      personMenuFor={raisePersonMenu}
      sort={refsFor("voice", sectionId)}
      canMoveOthers={canMoveOthers}
      carry={carry}
      arrives={arrives}
    />
  );

  const createChannel = canManageChannels ? onCreateChannel : undefined;
  const createAction = (kind: "text" | "voice") =>
    createChannel !== undefined ? (
      <CreateChannelButton
        label={t(
          kind === "text"
            ? "shell.createTextChannel"
            : "shell.createVoiceChannel",
        )}
        onCreate={() => createChannel(kind)}
      />
    ) : null;

  const fileChannel = channelMenu?.onMoveToSection;
  let openMenu: ReactNode = null;
  if (ctx.at !== null && menu?.type === "channel" && channelMenu !== undefined) {
    openMenu = (
      <ChannelMenu
        {...channelMenu}
        {...(fileChannel === undefined
          ? {}
          : {
              onMoveToSection: (channelId: string, sectionId: string | null) =>
                void rearrange(() => fileChannel(channelId, sectionId)),
            })}
        at={ctx.at}
        channelId={menu.channel.id}
        kind={menu.channel.kind}
        name={menu.channel.name}
        unread={menu.channel.unread}
        currentSectionId={menu.channel.sectionId}
        sectionName={sections.find((section) => section.id === menu.channel.sectionId)?.name}
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
        avatarUrl={menu.person.avatarUrl}
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
        onCreateChannel={!showTextCaption && !showVoiceCaption && createChannel !== undefined
          ? () => createChannel("text")
          : undefined}
        onEditServer={canAccessServerSettings ? onEditServer : undefined}
        onInvite={canManageServer ? onInvite : undefined}
      />

      <div className="rail-scroll" ref={dragContainer}>
        <CollapsibleGroup
          id={`${railId}-text`}
          caption={showTextCaption ? t("shell.textChannels") : undefined}
          captionTitle={canSort ? t("shell.dragChannel") : undefined}
          open={looseTextOpen}
          onToggle={() => setTextOpen((open) => !open)}
          actions={createAction("text")}
        >
          {textList(looseText, null)}
        </CollapsibleGroup>
        <CollapsibleGroup
          id={`${railId}-voice`}
          caption={showVoiceCaption ? t("shell.voiceChannels") : undefined}
          open={looseVoiceOpen}
          onToggle={() => setVoiceOpen((open) => !open)}
          actions={createAction("voice")}
        >
          {voiceList(looseVoice, null)}
        </CollapsibleGroup>

        {/* The sections are sorted as one list, under the loose groups. */}
        <div className="rail-sections" ref={sectionsList}>
          {grouped.map(({ section, text, voice }) => (
            <RailSection
              key={section.id}
              itemRef={sectionItem(section.id)}
              drag={sectionHandle?.(section.id)}
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
        </div>

        {moveError === null ? null : (
          <p className="rail-move-error" role="alert">
            {moveError}
          </p>
        )}
      </div>

      {dragLayer}
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
