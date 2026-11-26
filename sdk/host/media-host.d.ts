export interface HostMediaControls { stopCamera(): void; dispose(): void }
export function bindMediaHost(options: {
  frame: HTMLIFrameElement; origin: string; nonce?: string; youtube?: boolean; title: string; permissions?: string[]; controls: HTMLElement;
  volume: HTMLInputElement; muted?: () => boolean; fixtures?: boolean; language?: string | (() => string);
  tile(action: Record<string, unknown>): Promise<Record<string, string | null>>;
  /** Where camera images are drawn, over the frame; a positioned ancestor of it. The page's body without it. */
  overlay?: HTMLElement;
}): HostMediaControls;
