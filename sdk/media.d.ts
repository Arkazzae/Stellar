export interface DeviceHandle {
  stop(): void;
  onEnded(listener: (reason: string) => void): () => void;
}
export interface ActivityMedia {
  volume: { get(): number; subscribe(listener: (value: number) => void): () => void };
  microphoneLevel: { open(): Promise<DeviceHandle & { subscribe(listener: (level: number) => void): () => void }> };
  cameraTile: { open(): Promise<DeviceHandle>; mount(participantId: string, element: Element): () => void };
}
export function createMedia(options: {context: unknown; permissions?: string[]; volume?: number}): ActivityMedia & {dispose(): void};
