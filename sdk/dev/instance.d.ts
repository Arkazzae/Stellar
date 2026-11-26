import type { IncomingMessage, ServerResponse } from "node:http";
export function previewMiddleware(config: { origin: string; domain: string; apiOrigin: string; clock?: () => number }): {
  middleware(req: IncomingMessage, res: ServerResponse, next: () => void): void;
  close(): void;
};
