export interface HostMediaControls { stopCamera(): void; dispose(): void }
export function bindMediaHost(options: {
  frame: HTMLIFrameElement; origin: string; nonce?: string; youtube?: boolean; title: string; permissions?: string[]; controls: HTMLElement;
  volume: HTMLInputElement; muted?: () => boolean; fixtures?: boolean; language?: string | (() => string);
  tile(action: Record<string, unknown>): Promise<Record<string, string | null>>;
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
