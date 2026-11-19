import artifactSizes from "virtual:artifact-sizes";

import { usesWorkletSuppression, type WorkletSuppressor } from "../audioProfiles";
import type { NoiseSuppressionMode } from "../settings";
import type { SuppressorDownload } from "../types";
import {
  preloadSuppressor,
  watchSuppressorDownloads,
  type SuppressorDownloadEvent,
} from "./noiseSuppression";

/**
 * How long a download runs before it is shown. Speex's binary is about 32 KB
 * on the wire and is usually here before this; a ring that appeared and
 * vanished within a blink would only say that something had happened.
 */
export const DOWNLOAD_SHOWN_AFTER_MS = 400;

/** The size the manifest records for a served file, or undefined. */
function recordedSize(url: string): number | undefined {
  const size = artifactSizes[url.replace(/^\//, "")];
  return typeof size === "number" && size > 0 ? size : undefined;
}

export interface SuppressorDownloadsDeps {
  /** Receives what to show whenever it changes. */
  publish: (download: SuppressorDownload | null) => void;
  preload?: (kind: WorkletSuppressor) => Promise<unknown>;
  watch?: typeof watchSuppressorDownloads;
  sizeOf?: (url: string) => number | undefined;
}

/** One engine's binary on its way. */
interface Flight {
  url: string;
  percent: number | null;
  /** The received count passed the recorded size, so that size is not this file's. */
  overran: boolean;
  shown: boolean;
  timer: ReturnType<typeof setTimeout>;
}

/**
 * Starts the download of the engine a person settles on, and says how far the
 * chosen engine's download has come.
 *
 * It follows every binary download, whoever started it, one record per
 * engine: an engine left behind may still be finishing while the next one
 * starts, and only the chosen one is shown. A download is shown once it has run
 * for `DOWNLOAD_SHOWN_AFTER_MS`, and stops being shown the moment it ends,
 * whether the file arrived or not; a capture that then finds no binary falls
 * back to the browser's suppressor and says so, as it always has.
 */
export class SuppressorDownloads {
  private readonly flights = new Map<WorkletSuppressor, Flight>();
  private readonly stopWatching: () => void;
  private readonly preload: (kind: WorkletSuppressor) => Promise<unknown>;
  private readonly sizeOf: (url: string) => number | undefined;
  private shownNow: SuppressorDownload | null = null;

  constructor(
    private chosen: NoiseSuppressionMode,
    private readonly deps: SuppressorDownloadsDeps,
  ) {
    this.preload = deps.preload ?? preloadSuppressor;
    this.sizeOf = deps.sizeOf ?? recordedSize;
    this.stopWatching = (deps.watch ?? watchSuppressorDownloads)((event) => this.follow(event));
  }

  /** What is shown now: the chosen engine's download, or null. */
  get current(): SuppressorDownload | null {
    return this.shownNow;
  }

  /**
   * The mode the person settled on. An engine is fetched now, whether or not a
   * call is on, so the capture that needs it finds it here; one already
   * fetched costs nothing. The mode in effect when this was made is not
   * fetched: a choice stored from an earlier visit waits for a capture.
   */
  choose(mode: NoiseSuppressionMode): void {
    if (mode === this.chosen) return;
    this.chosen = mode;
    if (usesWorkletSuppression(mode)) void this.preload(mode);
    this.publish();
  }

  dispose(): void {
    this.stopWatching();
    for (const flight of this.flights.values()) clearTimeout(flight.timer);
    this.flights.clear();
    this.publish();
  }

  private follow(event: SuppressorDownloadEvent): void {
    const { kind } = event;
    if (event.phase !== "loading") {
      const flight = this.flights.get(kind);
      if (flight) clearTimeout(flight.timer);
      this.flights.delete(kind);
      this.publish();
      return;
    }

    let flight = this.flights.get(kind);
    if (!flight) {
      const started: Flight = {
        url: event.url,
        percent: null,
        overran: false,
        shown: false,
        timer: setTimeout(() => {
          started.shown = true;
          this.publish();
        }, DOWNLOAD_SHOWN_AFTER_MS),
      };
      flight = started;
      this.flights.set(kind, flight);
    }
    flight.percent = this.percentOf(flight, event.received);
    if (flight.shown) this.publish();
  }

  /**
   * Whole percent, from the size the manifest records. It never passes 100 and
   * never falls back. A file that grows past its recorded size has a total
   * nobody knows, so it is shown without one until it ends.
   */
  private percentOf(flight: Flight, received: number): number | null {
    const size = this.sizeOf(flight.url);
    if (size === undefined || flight.overran) return null;
    if (received > size) {
      flight.overran = true;
      return null;
    }
    return Math.max(flight.percent ?? 0, Math.floor((received * 100) / size));
  }

  private shown(): SuppressorDownload | null {
    const kind = this.chosen;
    if (!usesWorkletSuppression(kind)) return null;
    const flight = this.flights.get(kind);
    return flight?.shown ? { kind, percent: flight.percent } : null;
  }

  private publish(): void {
    const next = this.shown();
    const previous = this.shownNow;
    if (previous?.kind === next?.kind && previous?.percent === next?.percent) return;
    this.shownNow = next;
    this.deps.publish(next);
  }
}
