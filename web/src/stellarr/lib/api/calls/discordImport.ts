import {
  DiscordImportRefusal,
  type DiscordImportPort,
  type DiscordImportStatus,
  type DiscordPreview,
  type DiscordPreviewChannel,
  type DiscordSourceServer,
} from "@/ports";

import {
  expectBoolean, expectRecord, expectSafeInteger, expectString, nullableString,
} from "../expect";
import { ApiError } from "../errors";
import { getJson, parseList, route, sendJson, sendVoid, type Parse } from "./http";

const IMPORTS = "/api/servers/discord/imports";

const parseSource: Parse<DiscordSourceServer> = (value, path) => {
  const source = expectRecord(value, path);
  return {
    id: expectString(source, "id", path),
    name: expectString(source, "name", path),
  };
};

const parseBotServers: Parse<{ servers: DiscordSourceServer[]; installUrl: string }> = (value, path) => {
  const source = expectRecord(value, path);
  const installUrl = expectString(source, "installUrl", path);
  if (!installUrl.startsWith("https://discord.com/oauth2/authorize?")) throw new ApiError("malformed", path);
  return { servers: parseList(parseSource)(source.servers, path), installUrl };
};

const parseChannel: Parse<DiscordPreviewChannel> = (value, path) => {
  const source = expectRecord(value, path);
  const kind = expectString(source, "kind", path);
  if (kind !== "text" && kind !== "voice") throw new ApiError("malformed", path);
  return {
    ...parseSource(source, path),
    kind,
    sectionId: nullableString(source, "sectionId", path),
    canCopyHistory: expectBoolean(source, "canCopyHistory", path),
    hasOwnMessages: expectBoolean(source, "hasOwnMessages", path),
  };
};

const parsePreview: Parse<DiscordPreview> = (value, path) => {
  const source = expectRecord(value, path);
  return {
    name: expectString(source, "name", path),
    messageContent: expectBoolean(source, "messageContent", path),
    sections: parseList(parseSource)(source.sections, path),
    channels: parseList(parseChannel)(source.channels, path),
    restrictedChannels: expectSafeInteger(source, "restrictedChannels", path),
  };
};

const parseStatus: Parse<DiscordImportStatus> = (value, path) => {
  const source = expectRecord(value, path);
  const state = expectString(source, "state", path);
  if (state !== "running" && state !== "completed" && state !== "failed") {
    throw new ApiError("malformed", path);
  }
  const report = source.report == null ? null : expectRecord(source.report, path);
  return {
    id: expectString(source, "id", path),
    state,
    serverId: nullableString(source, "serverId", path),
    name: expectString(source, "name", path),
    totalSources: source.totalSources == null ? null : expectSafeInteger(source, "totalSources", path),
    completedSources: expectSafeInteger(source, "completedSources", path),
    channelsImported: expectSafeInteger(source, "channelsImported", path),
    sectionsImported: expectSafeInteger(source, "sectionsImported", path),
    historyRequested: expectBoolean(source, "historyRequested", path),
    pendingAttachments: source.pendingAttachments == null ? 0 : expectSafeInteger(source, "pendingAttachments", path),
    error: nullableString(source, "error", path),
    report: report && {
      messages: expectSafeInteger(report, "messages", path),
      skippedMessages: expectSafeInteger(report, "skippedMessages", path),
      messagesMissingAttachments: expectSafeInteger(report, "messagesMissingAttachments", path),
    },
  };
};

async function withImportErrors<T>(response: Promise<T>): Promise<T> {
  try {
    return await response;
  } catch (cause) {
    if (cause instanceof ApiError) throw new DiscordImportRefusal(cause.status, cause.code);
    throw cause;
  }
}

export const discordImport: DiscordImportPort = {
  servers: (botToken, signal) =>
    withImportErrors(sendJson("POST", "/api/servers/discord/servers", { botToken }, parseBotServers, signal)),
  preview: (botToken, sourceId, signal) =>
    withImportErrors(sendJson("POST", "/api/servers/discord/preview", { botToken, sourceId }, parsePreview, signal)),
  start: (id, botToken, options, signal) =>
    withImportErrors(sendJson("POST", IMPORTS, { id, botToken, options }, parseStatus, signal)),
  status: (id, signal) =>
    withImportErrors(getJson(route`/api/servers/discord/imports/${id}`, parseStatus, signal)),
  latest: (signal) =>
    withImportErrors(getJson("/api/servers/discord/imports/latest",
      (value, path) => value === null ? null : parseStatus(value, path), signal)),
  resume: (id, botToken, signal) =>
    withImportErrors(sendJson("POST", route`/api/servers/discord/imports/${id}/resume`, { botToken }, parseStatus, signal)),
  discard: (id, signal) =>
    withImportErrors(sendVoid("DELETE", route`/api/servers/discord/imports/${id}`, undefined, signal)),
};
