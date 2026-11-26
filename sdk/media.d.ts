export interface DeviceHandle {
  stop(): void;
  onEnded(listener: (reason: string) => void): () => void;
}
export interface ActivityMedia {
  volume: { get(): number; subscribe(listener: (value: number) => void): () => void };
  youtube?: { mount(videoId: string, element: Element, events: {
    ready(): void; stateChange(state: number): void; blocked(): void; error(code: number): void;
  }): {
    prepare(position: number, playing: boolean): void; time(): number; duration(): number; running(): boolean;
    fullscreen(): void; play(): void; pause(): void; seek(position: number): void; volume(value: number): void; destroy(): void;
  } };
  microphoneLevel: { open(): Promise<DeviceHandle & { subscribe(listener: (level: number) => void): () => void }> };
  cameraTile: { open(): Promise<DeviceHandle>; mount(participantId: string, element: Element): () => void };
}
export function createMedia(options: {context: unknown; permissions?: string[]; volume?: number; youtube?: boolean}): ActivityMedia & {dispose(): void};
