import { useCallback, useEffect, useRef, useState, type CSSProperties } from "react";

import { Icon } from "@stellarr/icons";
import { Avatar } from "@/components/primitives/Avatar";
import type { PersonVM, ScreenShareVM } from "@/components/view-models";
import { useLanguage } from "@/state/LanguageProvider";

import { ParticipantTile } from "./ParticipantTile";
import { participantTileProps, type ParticipantTilesProps } from "./ParticipantTiles";
import { ScreenTile } from "./ScreenTile";
import { useChromeIdle } from "./useChromeIdle";
import { useTileGrid } from "./useTileGrid";

export interface VoiceStageProps extends ParticipantTilesProps {
  /** Every screen in the call, this user's own first. */
  shares: readonly ScreenShareVM[];
  /** Who watches each screen, by its key, when the call knows. */
  viewersOf?: (shareKey: string) => readonly PersonVM[];
  onWatchShare: (key: string) => void;
  onStopWatchingShare: () => void;
  /** Stops this user's own share, from its tile. */
  onStopSharing?: () => void;
  /**
   * Cinema mode. The stage hides the strip of people; the matching
   * `.stage-full` on the shell (which also collapses the roster) is the
   * shell's business, so the flag is passed in rather than owned here.
   */
  cinema?: boolean;
}

/** How long an ended share keeps its tile to say so. */
const ENDED_MS = 3000;
const STRIP_KEY = "stellarr_stage_strip_hidden";

type StageTile =
  | { kind: "screen"; key: string; share: ScreenShareVM; ended: boolean }
  | { kind: "person"; key: string; person: PersonVM };

const personKey = (id: string) => `person:${id}`;

function readStripHidden(): boolean {
  try {
    return window.localStorage.getItem(STRIP_KEY) === "1";
  } catch {
    return false;
  }
}

function writeStripHidden(hidden: boolean) {
  try {
    if (hidden) window.localStorage.setItem(STRIP_KEY, "1");
    else window.localStorage.removeItem(STRIP_KEY);
  } catch {
    // A locked-down browser keeps the choice for this visit only.
  }
}

/**
 * Remote shares that ended a moment ago, kept as tiles for a few seconds so
 * the stage says what happened instead of a screen vanishing. Your own share
 * leaves at once: you stopped it yourself.
 */
function useEndedShares(shares: readonly ScreenShareVM[]): ScreenShareVM[] {
  const [ended, setEnded] = useState<ScreenShareVM[]>([]);
  const previous = useRef<readonly ScreenShareVM[]>(shares);
  const timers = useRef(new Map<string, ReturnType<typeof setTimeout>>());

  useEffect(() => {
    const before = previous.current;
    previous.current = shares;
    const live = new Set(shares.map((share) => share.key));
    const gone = before.filter((share) => !share.local && !live.has(share.key));
    if (gone.length === 0) return;
    setEnded((current) => [
      ...current.filter((share) => !gone.some((left) => left.key === share.key)),
      ...gone,
    ]);
    for (const share of gone) {
      const timer = setTimeout(() => {
        timers.current.delete(share.key);
        setEnded((current) => current.filter((item) => item.key !== share.key));
      }, ENDED_MS);
      timers.current.set(share.key, timer);
    }
  }, [shares]);

  useEffect(() => {
    const pending = timers.current;
    return () => {
      for (const timer of pending.values()) clearTimeout(timer);
    };
  }, []);

  return ended;
}

/**
 * The call's stage: every person and every shared screen as a tile.
 *
 * As a grid the tiles share the room evenly. A tile given the focus (a screen
 * you start to watch, or anyone you click) takes the stage, and the rest line
 * up in a strip under it, which can be hidden. Both layouts are one list of
 * tiles told apart by classes, so a picture is never mounted anew when the
 * layout changes.
 */
export function VoiceStage({
  shares,
  viewersOf,
  onWatchShare,
  onStopWatchingShare,
  onStopSharing,
  cinema = false,
  people,
  ...sources
}: VoiceStageProps) {
  const { t } = useLanguage();
  const [focusKey, setFocusKey] = useState<string | null>(null);
  const [stripHidden, setStripHidden] = useState(readStripHidden);
  const [previewHidden, setPreviewHidden] = useState(false);
  const chrome = useChromeIdle();
  const ended = useEndedShares(shares);

  const tiles: StageTile[] = [
    ...shares.map((share): StageTile => ({ kind: "screen", key: share.key, share, ended: false })),
    ...ended.map((share): StageTile => ({ kind: "screen", key: share.key, share, ended: true })),
    ...people.map((person): StageTile => ({ kind: "person", key: personKey(person.id), person })),
  ];
  const focused = tiles.find((tile) => tile.key === focusKey) ?? null;
  const layout = focused === null ? "grid" : "focus";
  const grid = useTileGrid(layout === "grid" ? tiles.length : 1);

  const toggleStrip = useCallback(() => {
    setStripHidden((hidden) => {
      writeStripHidden(!hidden);
      return !hidden;
    });
  }, []);

  const focus = (key: string) => setFocusKey((current) => (current === key ? null : key));
  const showGrid = () => setFocusKey(null);
  const watch = (key: string) => {
    onWatchShare(key);
    setFocusKey(key);
  };
  const stopWatching = () => {
    onStopWatchingShare();
    setFocusKey(null);
  };

  const strip = tiles.filter((tile) => tile !== focused);
  const speakers = people.filter((person) => person.speaking);

  return (
    <div
      className={["voice-stage", cinema && "cinema"].filter(Boolean).join(" ")}
      data-layout={layout}
      data-strip={stripHidden || cinema ? "hidden" : "shown"}
      data-chrome={chrome.idle ? "idle" : "awake"}
      onPointerMove={chrome.wake}
      onPointerDown={chrome.wake}
      onKeyDown={chrome.wake}
    >
      <div
        ref={grid}
        className="tiles"
        style={{ "--strip-count": Math.max(strip.length, 1) } as CSSProperties}
      >
        {tiles.map((tile) =>
          tile.kind === "screen" ? (
            <ScreenTile
              key={tile.key}
              share={tile.share}
              ended={tile.ended}
              focused={tile === focused}
              viewers={viewersOf?.(tile.key)}
              volume={sources.participantVolumes?.get(tile.share.ownerId)}
              onVolumeChange={
                sources.onParticipantVolumeChange &&
                ((volume) => sources.onParticipantVolumeChange?.(tile.share.ownerId, volume))
              }
              previewHidden={tile.share.local && previewHidden}
              onTogglePreview={tile.share.local ? () => setPreviewHidden((hidden) => !hidden) : undefined}
              onStopSharing={tile.share.local ? onStopSharing : undefined}
              onWatch={() => watch(tile.key)}
              onStopWatching={stopWatching}
              onFocus={() => focus(tile.key)}
              onShowGrid={showGrid}
            />
          ) : (
            <ParticipantTile
              key={tile.key}
              {...participantTileProps(tile.person, sources)}
              focused={tile === focused}
              onFocus={() => focus(tile.key)}
            />
          ),
        )}

        {focused === null ? null : (
          <div className="stage-chrome">
            {focused.kind === "person" ? (
              <button type="button" className="st-chip st-button stage-grid" onClick={showGrid}>
                <Icon name="apps" size={15} />
                {t("stage.grid")}
              </button>
            ) : null}
            {cinema || strip.length === 0 ? null : (
              <button
                type="button"
                className="st-chip st-button stage-handle"
                aria-expanded={!stripHidden}
                onClick={toggleStrip}
              >
                <Icon name={stripHidden ? "caretUp" : "caret"} size={14} />
                {stripHidden ? t("stage.showPeople") : t("stage.hidePeople")}
                {stripHidden ? (
                  <span className="stage-faces" aria-hidden="true">
                    {(speakers.length > 0 ? speakers : people).slice(0, 4).map((person) => (
                      <span
                        key={person.id}
                        className={person.speaking ? "stage-face speaking" : "stage-face"}
                      >
                        <Avatar
                          id={person.id}
                          displayName={person.displayName}
                          initials={person.initials}
                          avatarUrl={person.avatarUrl}
                          size="xs"
                        />
                      </span>
                    ))}
                  </span>
                ) : null}
              </button>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
