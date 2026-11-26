export interface HostMediaControls { stopCamera(): void; dispose(): void }
export function bindMediaHost(options: {
  frame: HTMLIFrameElement; origin: string; title: string; permissions?: string[]; controls: HTMLElement;
  volume: HTMLInputElement; muted?: () => boolean; fixtures?: boolean; language?: string;
  tile(action: Record<string, unknown>): Promise<Record<string, string | null>>;
}): HostMediaControls;
