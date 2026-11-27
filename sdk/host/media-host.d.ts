export interface HostMediaControls { stopCamera(): void; dispose(): void }
/** Owned by the parent. Tracks and streams never cross the iframe port. */
export interface ActivityCamera {
  publish(stream: MediaStream, signal: AbortSignal, ended: () => void): Promise<void>;
  watch(participant: string, show: (stream: MediaStream | null) => void): () => void;
  dispose(): void;
}
/**
 * GIF search the page supplies; the frame never reaches the source itself.
 * Each call may reject with an error whose `code` is a `GifErrorCode` from
 * `media.js`; anything else reaches the frame as `failed`. The host passes on
 * only the fields below.
 */
export interface ActivityGifs {
  search(request: { query: string; offset: number; limit: number }, signal: AbortSignal): Promise<{
    items: { id: string; alt?: string; width: number; height: number }[]; offset: number; total: number;
  }>;
  load(request: { gif: string; size: "preview" | "full"; maxBytes: number }, signal: AbortSignal): Promise<{
    bytes: ArrayBuffer; type: string; width: number; height: number; alt?: string;
  }>;
}
/** What one frame may ask of `gifs`, enforced before the provider is called. */
export const gifLimits: {
  searchesPerMinute: number; loadsPerMinute: number; concurrentLoads: number; queuedLoads: number;
  bytes: Record<"preview" | "full", number>; pageSize: number; queryLength: number; maxOffset: number;
};
export function bindMediaHost(options: {
  frame: HTMLIFrameElement; origin: string; nonce?: string; embedHosts?: string[]; title: string; permissions?: string[]; controls: HTMLElement;
  volume: HTMLInputElement; muted?: () => boolean; fixtures?: boolean; language?: string | (() => string);
  camera?: () => ActivityCamera;
  cloneLocalCamera?: () => MediaStreamTrack | undefined;
  /** Image relay for standalone development previews. */
  tile?(action: Record<string, unknown>): Promise<Record<string, string | null>>;
  /** Answers `media.gifs`; without it the activity hears `unavailable`. */
  gifs?: ActivityGifs;
  /** Where camera images are drawn, over the frame; a positioned ancestor of it. The page's body without it. */
  overlay?: HTMLElement;
  /**
   * Asks the person before a device opens, in the page's own words; resolves
   * true to allow. The signal aborts when the activity withdraws the request.
   * Without it the host asks in a dialog of its own.
   */
  ask?(kind: "cameraTile" | "microphoneLevel", signal: AbortSignal): Promise<boolean>;
  /** The page's own words for what the host draws. */
  labels?: { stop?: Partial<Record<"cameraTile" | "microphoneLevel", string>>; camera?: string };
}): HostMediaControls;
