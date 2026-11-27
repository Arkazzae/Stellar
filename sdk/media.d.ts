export interface DeviceHandle {
  stop(): void;
  onEnded(listener: (reason: string) => void): () => void;
}
export interface CameraTileMount {
  (): void;
  setVisible(visible: boolean): void;
}
export interface EmbedOptions {
  title?: string;
  onLoad?(): void;
  onMessage?(data: unknown): void;
}
export interface EmbedHandle {
  setVisible(visible: boolean): void;
  postMessage(data: unknown): void;
  destroy(): void;
}
/**
 * Why a `media.gifs` call was refused, as `error.code`: `permission` (the
 * manifest lacks `media.gifs`), `unavailable` (the instance offers no GIFs),
 * `declined` (this player turned GIPHY off), `rateLimited` (this frame or the
 * instance's key asked too often), `notFound` (no such GIF) or `failed`.
 */
export type GifErrorCode = "permission" | "unavailable" | "declined" | "rateLimited" | "notFound" | "failed";
export interface GifError extends Error { code: GifErrorCode }
export interface GifItem { id: string; alt?: string; width: number; height: number }
export interface GifPage { items: GifItem[]; offset: number; total: number }
export interface GifImage {
  /** A `blob:` URL made in this frame from bytes the host sent. */
  url: string;
  type: string;
  width: number;
  height: number;
  alt?: string;
  /** Revokes `url`. Closing the session revokes the rest. */
  release(): void;
}
export interface ActivityMedia {
  volume: { get(): number; subscribe(listener: (value: number) => void): () => void };
  embed: { mount(url: string, element: Element, options?: EmbedOptions): EmbedHandle };
  microphoneLevel: { open(): Promise<DeviceHandle & { subscribe(listener: (level: number) => void): () => void }> };
  cameraTile: { open(): Promise<DeviceHandle>; mount(participantId: string, element: Element): CameraTileMount };
  /** Requires `media.gifs`. An empty query lists trending GIFs. Rejections are `GifError`s. */
  gifs: {
    search(query?: string, options?: { offset?: number; limit?: number }): Promise<GifPage>;
    load(id: string, size?: "preview" | "full"): Promise<GifImage>;
  };
}
export function createMedia(options: {context: unknown; permissions?: string[]; volume?: number; embedHosts?: string[]; hostedEmbeds?: boolean}): ActivityMedia & {dispose(): void};
/** Used by the host bootstrap to mount a frame in the provider’s parent window. */
export function attachEmbed(container: Element, url: URL, options: EmbedOptions): EmbedHandle;
