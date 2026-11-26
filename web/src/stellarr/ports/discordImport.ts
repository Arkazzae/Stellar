export interface DiscordSourceServer {
  id: string;
  name: string;
}

export interface DiscordPreviewChannel extends DiscordSourceServer {
  kind: "text" | "voice";
  sectionId: string | null;
  canCopyHistory: boolean;
  hasOwnMessages: boolean;
}

export interface DiscordPreview {
  name: string;
  messageContent: boolean;
  sections: DiscordSourceServer[];
  channels: DiscordPreviewChannel[];
  restrictedChannels: number;
}

export type DiscordHistory = { type: "structure" | "all" } | { type: "since"; since: string };

export interface DiscordImportOptions {
  sourceId: string;
  name: string;
  channelIds: string[];
  history: DiscordHistory;
  includeThreads: boolean;
}

export interface DiscordImportStatus {
  id: string;
  state: "running" | "completed" | "failed";
  serverId: string | null;
  name: string;
  totalSources: number | null;
  completedSources: number;
  channelsImported: number;
  sectionsImported: number;
  historyRequested: boolean;
  pendingAttachments: number;
  error: string | null;
  report: {
    messages: number;
    skippedMessages: number;
    messagesMissingAttachments: number;
  } | null;
}

export class DiscordImportRefusal extends Error {
  constructor(readonly status: number, readonly code?: string) {
    super("Discord import request failed");
    this.name = "DiscordImportRefusal";
  }
}

export interface DiscordImportPort {
  servers(botToken: string, signal?: AbortSignal): Promise<{ servers: DiscordSourceServer[]; installUrl: string }>;
  preview(botToken: string, sourceId: string, signal?: AbortSignal): Promise<DiscordPreview>;
  start(id: string, botToken: string, options: DiscordImportOptions, signal?: AbortSignal): Promise<DiscordImportStatus>;
  status(id: string, signal?: AbortSignal): Promise<DiscordImportStatus>;
  latest(signal?: AbortSignal): Promise<DiscordImportStatus | null>;
  resume(id: string, botToken: string, signal?: AbortSignal): Promise<DiscordImportStatus>;
  discard(id: string, signal?: AbortSignal): Promise<void>;
}
