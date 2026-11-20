import { useId, useLayoutEffect, useRef, type ReactNode } from "react";

import { Button } from "@stellarr/ui";
import { Icon, type IconName } from "@stellarr/icons";
import { countedNoun } from "@/components/settings/paneFormat";
import { SettingsSection } from "@/components/settings/SettingsSection";
import type { ChannelMove } from "@/components/view-models";
import { DragAnnouncer } from "@/lib/drag/DragAnnouncer";
import { useRailDrag, type DragHandleProps, type SortList, type SortPlace } from "@/lib/drag/useRailDrag";
import { useAsyncAction } from "@/hooks/useAsyncAction";
import type { MessageKey } from "@/i18n";
import { useLanguage } from "@/state/hooks";

export interface SettingsChannelVM {
  id: string;
  name: string;
  kind: "text" | "voice";
  /** Which section owns it; channels without one are listed first. */
  sectionId?: string;
}

export interface SettingsSectionVM {
  id: string;
  name: string;
}

export interface ChannelsPaneProps {
  /** False when this member may not manage channels: no handles, no edits. */
  supported: boolean;
  /** Every channel, in the order the rail shows them. */
  channels: readonly SettingsChannelVM[];
  onAdd: () => void;
  onEdit: (channelId: string) => void;
  /** False when this member may not manage sections. */
  sectionsSupported: boolean;
  sections: readonly SettingsSectionVM[];
  onAddSection: () => void;
  onEditSection: (sectionId: string) => void;
  /** The same move the rail makes when a channel is dropped. */
  onMoveChannel: (move: ChannelMove) => Promise<void>;
  /** The sections' new order. */
  onMoveSection: (order: string[]) => Promise<void>;
}

/** The rail's own glyphs, so a channel looks the same in both places. */
const KIND_ICON: Record<SettingsChannelVM["kind"], IconName> = { text: "hash", voice: "speaker" };

const SECTIONS = "sections";
const listId = (kind: SettingsChannelVM["kind"], sectionId: string | null) => `${kind}:${sectionId ?? ""}`;

interface PaneList extends SortList {
  sectionId: string | null;
  name: string;
}

/**
 * The channels in their sections, as one list. A channel is dragged by its
 * handle within its section or into another one, and a section moves with
 * its channels, the way the rail sorts them: text and voice channels keep
 * their own lists inside each group, as they do there.
 */
export function ChannelsPane({
  supported,
  channels,
  onAdd,
  onEdit,
  sectionsSupported,
  sections,
  onAddSection,
  onEditSection,
  onMoveChannel,
  onMoveSection,
}: ChannelsPaneProps) {
  const { locale, t } = useLanguage();
  const move = useAsyncAction();
  const channelInstructions = useId();
  const sectionInstructions = useId();
  const listRef = useRef<HTMLDivElement>(null);

  const inGroup = (kind: SettingsChannelVM["kind"], sectionId: string | null) =>
    channels.filter((channel) => channel.kind === kind && (channel.sectionId ?? null) === sectionId);
  const ids = (list: readonly { id: string }[]) => list.map((item) => item.id);
  const lists: PaneList[] = [
    { id: listId("text", null), kind: "text", items: ids(inGroup("text", null)), closed: false, sectionId: null, name: t("srv.channels.loose") },
    { id: listId("voice", null), kind: "voice", items: ids(inGroup("voice", null)), closed: false, sectionId: null, name: t("srv.channels.loose") },
    ...sections.flatMap((section) => [
      { id: listId("text", section.id), kind: "text", items: ids(inGroup("text", section.id)), closed: false, sectionId: section.id, name: section.name },
      { id: listId("voice", section.id), kind: "voice", items: ids(inGroup("voice", section.id)), closed: false, sectionId: section.id, name: section.name },
    ]),
    { id: SECTIONS, kind: "section", items: ids(sections), closed: false, sectionId: null, name: "" },
  ];
  const listOf = (id: string) => lists.find((list) => list.id === id);
  const sectionName = (id: string) => sections.find((section) => section.id === id)?.name;
  const channelName = (id: string) => channels.find((channel) => channel.id === id)?.name ?? "";
  const said = (channel: MessageKey, section: MessageKey) => (id: string, place: SortPlace) =>
    t(place.list === SECTIONS ? section : channel, {
      name: sectionName(id) ?? channelName(id),
      position: place.position,
      count: place.count,
      group: listOf(place.list)?.name ?? "",
    });

  const { container, list, item, handle, announcer } = useRailDrag({
    lists,
    onMove: ({ id, list: target, order }) => {
      void move.run(() =>
        target === SECTIONS
          ? onMoveSection(order)
          : onMoveChannel({ channelId: id, sectionId: listOf(target)?.sectionId ?? null, order }),
      );
    },
    messages: {
      lifted: said("shell.reorderLifted", "shell.sectionLifted"),
      moved: said("shell.reorderMoved", "shell.sectionMoved"),
      dropped: said("shell.reorderDropped", "shell.sectionDropped"),
      cancelled: (id) => t(sectionName(id) === undefined ? "shell.reorderCancelled" : "shell.sectionCancelled"),
    },
  });

  // The pane column scrolls, not the card, so that is where a drag scrolls
  // at the edges and measures from.
  useLayoutEffect(() => {
    const scroller = listRef.current?.closest<HTMLElement>(".sv-panes") ?? listRef.current;
    container(scroller ?? null);
    return () => container(null);
  }, [container]);

  const channelHandle = (id: string): DragHandleProps | undefined =>
    supported ? handle(id, { roleDescription: t("shell.reorderHandle"), instructionsId: channelInstructions }) : undefined;
  const sectionHandle = (id: string): DragHandleProps | undefined =>
    supported && sectionsSupported
      ? handle(id, { roleDescription: t("shell.sectionHandle"), instructionsId: sectionInstructions })
      : undefined;

  const row = (channel: SettingsChannelVM) => (
    <div className="sec-row ss-ch" key={channel.id} ref={item(channel.id)} data-channel={channel.id}>
      <Grip handle={channelHandle(channel.id)} label={t("srv.channels.moveChannel", { name: channel.name })} />
      <span className="ss-glyph" aria-hidden="true">
        <Icon name={KIND_ICON[channel.kind]} size={16} />
      </span>
      <span className="sec-copy">
        <b>{channel.name}</b>
        <small>{channel.kind === "text" ? t("channel.text") : t("channel.voice")}</small>
      </span>
      <Button
        type="button"
        variant="secondary"
        size="sm"
        disabled={!supported}
        aria-label={t("settings.editChannel", { name: channel.name })}
        onClick={() => onEdit(channel.id)}
      >
        {t("common.edit")}
      </Button>
    </div>
  );

  const kindLists = (sectionId: string | null, empty: ReactNode) => {
    const text = inGroup("text", sectionId);
    const voice = inGroup("voice", sectionId);
    return (
      <>
        <div className="ss-kind" ref={list(listId("text", sectionId))}>
          {text.map(row)}
        </div>
        <div className="ss-kind" ref={list(listId("voice", sectionId))}>
          {voice.map(row)}
        </div>
        {text.length + voice.length === 0 ? <p className="ss-kind-empty">{empty}</p> : null}
      </>
    );
  };

  return (
    <>
      {supported ? null : <p className="set-note">{t("settings.unsupported")}</p>}
      <SettingsSection
        title={t("srv.channels.title")}
        count={channels.length}
        countLabel={countedNoun(t, locale, "settings.channel", channels.length)}
        bodyClassName="sec-list ss-list"
      >
        <DragAnnouncer
          instructions={[
            { id: channelInstructions, text: t("shell.reorderInstructions") },
            { id: sectionInstructions, text: t("shell.sectionInstructions") },
          ]}
          announcer={announcer}
        />
        {move.failed ? (
          <p className="set-note sound-error" role="alert">
            {t("srv.channels.moveFailed")}
          </p>
        ) : null}
        <div className="ss-chan-list" ref={listRef}>
          <div className="ss-group ss-loose">
            <div className="ss-cap">{t("srv.channels.loose")}</div>
            {kindLists(null, t("srv.channels.looseEmpty"))}
          </div>
          <div className="ss-sections" ref={list(SECTIONS)}>
            {sections.map((section) => {
              const count = channels.filter((channel) => channel.sectionId === section.id).length;
              return (
                <div className="ss-group" key={section.id} ref={item(section.id)} data-section={section.id}>
                  <div className="sec-row ss-sec">
                    <Grip
                      handle={sectionHandle(section.id)}
                      label={t("srv.channels.moveSection", { name: section.name })}
                    />
                    <span className="sec-copy">
                      <b>{section.name}</b>
                      <small>{countedNoun(t, locale, "settings.channel", count)}</small>
                    </span>
                    <Button
                      type="button"
                      variant="secondary"
                      size="sm"
                      disabled={!sectionsSupported}
                      aria-label={t("settings.editSection", { name: section.name })}
                      onClick={() => onEditSection(section.id)}
                    >
                      {t("common.edit")}
                    </Button>
                  </div>
                  {kindLists(section.id, t("srv.channels.sectionEmpty"))}
                </div>
              );
            })}
          </div>
        </div>
        <div className="sec-row ss-foot">
          <span className="ss-foot-note">{supported ? t("srv.channels.note") : null}</span>
          <Button type="button" variant="secondary" size="sm" disabled={!sectionsSupported} onClick={onAddSection}>
            <Icon name="plus" size={15} />
            {t("srv.channels.newSection")}
          </Button>
          <Button type="button" variant="secondary" size="sm" disabled={!supported} onClick={onAdd}>
            <Icon name="plus" size={15} />
            {t("srv.channels.newChannel")}
          </Button>
        </div>
      </SettingsSection>
    </>
  );
}

/** The handle a row is carried by; nothing for a reader who may not sort. */
function Grip({ handle, label }: { handle: DragHandleProps | undefined; label: string }) {
  if (handle === undefined) return null;
  return (
    <button type="button" className="ss-grip" aria-label={label} title={label} {...handle}>
      <Icon name="grip" size={16} />
    </button>
  );
}
