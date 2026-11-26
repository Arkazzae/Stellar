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
export interface ActivityMedia {
  volume: { get(): number; subscribe(listener: (value: number) => void): () => void };
  embed: { mount(url: string, element: Element, options?: EmbedOptions): EmbedHandle };
  microphoneLevel: { open(): Promise<DeviceHandle & { subscribe(listener: (level: number) => void): () => void }> };
  cameraTile: { open(): Promise<DeviceHandle>; mount(participantId: string, element: Element): CameraTileMount };
}
export function createMedia(options: {context: unknown; permissions?: string[]; volume?: number; embedHosts?: string[]; hostedEmbeds?: boolean}): ActivityMedia & {dispose(): void};
/** Used by the host bootstrap to mount a frame in the provider’s parent window. */
export function attachEmbed(container: Element, url: URL, options: EmbedOptions): EmbedHandle;
