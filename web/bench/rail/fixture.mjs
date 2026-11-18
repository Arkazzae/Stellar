// The server the rail benchmark signs in to, and the API that answers for it.
// Nothing here talks to a real Stellarr: the browser's requests to /api are
// answered from this file, and the hub is refused, so the client runs as it
// does while its connection is down.

export const SERVER_ID = "00000000-0000-4000-8000-0000000000aa";
export const SELF_ID = "00000000-0000-4000-8000-000000000001";

const PERMISSIONS = [
  "manageServer",
  "manageChannels",
  "manageRoles",
  "manageMembers",
  "manageMessages",
  "moderateMembers",
  "manageBans",
];

const hex = (value, width = 12) => value.toString(16).padStart(width, "0");
const userId = (index) => `00000000-0000-4000-8000-${hex(index + 1)}`;
const channelId = (index) => `00000000-0000-4000-9000-${hex(index)}`;
const sectionId = (index) => `00000000-0000-4000-a000-${hex(index)}`;

const user = (index, name) => ({
  id: userId(index),
  username: name.toLowerCase().replace(/\s+/g, "-"),
  displayName: name,
  presence: "online",
});

/**
 * `sections` sections of `text` text and `voice` voice channels each, after
 * the unsectioned groups. The defaults make 150 channels: 15 text and 5 voice
 * outside sections, then ten sections of ten text and three voice. Every
 * group stays under the 600 px a rail group shows when open.
 */
export function buildServer({
  sections = 10,
  text = 10,
  voice = 3,
  looseText = 15,
  looseVoice = 5,
} = {}) {
  const self = user(0, "Bench Owner");
  const members = [self, ...Array.from({ length: 40 }, (_, index) => user(index + 1, `Member ${index + 1}`))];
  const channels = [];
  const add = (kind, name, section) =>
    channels.push({
      id: channelId(channels.length),
      name,
      kind,
      position: channels.length,
      topic: "",
      slowModeSeconds: 0,
      hideAfterInactivityHours: 0,
      audioProfile: "balanced",
      userLimit: 0,
      sectionId: section ?? null,
    });

  for (let index = 0; index < looseText; index += 1) add("text", `text-${index + 1}`);
  for (let index = 0; index < looseVoice; index += 1) add("voice", `voice-${index + 1}`);
  const sectionList = Array.from({ length: sections }, (_, index) => ({
    id: sectionId(index),
    name: `Section ${index + 1}`,
    position: index,
  }));
  for (const section of sectionList) {
    const number = section.position + 1;
    for (let index = 0; index < text; index += 1) add("text", `s${number}-text-${index + 1}`, section.id);
    for (let index = 0; index < voice; index += 1) add("voice", `s${number}-voice-${index + 1}`, section.id);
  }

  // Every other voice channel seats one or two people, so the voice rows
  // differ in height the way a busy evening makes them.
  const voiceIds = channels.filter((channel) => channel.kind === "voice").map((channel) => channel.id);
  const occupancy = Object.fromEntries(
    voiceIds.map((id, index) => [
      id,
      index % 2 === 1
        ? []
        : members.slice(1 + (index % 30), 2 + (index % 30) + (index % 4 === 0 ? 1 : 0)).map((member) => ({
            userId: member.id,
            displayName: member.displayName,
            muted: index % 3 === 0,
          })),
    ]),
  );

  return {
    user: self,
    server: {
      id: SERVER_ID,
      name: "Rail bench",
      description: "",
      iconUrl: null,
      createdAt: "2026-01-01T00:00:00Z",
      isPublic: false,
    },
    servers: [],
    channels,
    sections: sectionList,
    members,
    permissions: PERMISSIONS,
    messages: [],
    channelReads: {},
    voice: occupancy,
  };
}

const ACCOUNT_PREFERENCES = {
  desktopNotifications: false,
  mentionPopups: false,
  callRingtone: false,
  emailNotifications: false,
  pushNotifications: false,
  mentionsOnly: true,
  messageSound: false,
  joinLeaveSound: false,
  allowDirectMessages: true,
  allowFriendRequests: true,
  showPresence: true,
  developerMode: false,
  reducedMotion: false,
  mutedChannelIds: [],
};

/**
 * The answer to one request, as `{ status, body }`. `requests` collects every
 * write the client sends, so a run can check what a drop asked for.
 */
export function createApi(server, requests = []) {
  const json = (body, status = 200) => ({ status, body });
  const empty = (status = 204) => ({ status });

  return {
    requests,
    answer(method, url, postData) {
      const { pathname } = new URL(url);
      if (method !== "GET") {
        requests.push({ method, path: pathname, body: postData ? JSON.parse(postData) : undefined });
        return empty();
      }
      switch (pathname) {
        case "/api/bootstrap":
          return json(server);
        case "/api/session":
          return json({ user: server.user });
        case "/api/me":
          return json({
            ...server.user,
            createdAt: "2026-01-01T00:00:00Z",
            email: "bench@example.invalid",
            preferences: ACCOUNT_PREFERENCES,
          });
        case "/api/me/settings":
          return json({});
        case "/api/social":
          return json({ friends: [], requests: [], blocked: [], conversations: [] });
        case "/api/soundboard":
          return json({ sounds: [], settings: {}, canManage: false });
        case "/api/emoji":
          return json([]);
        case "/api/notifications":
          return json({ items: [], unread: 0 });
        default:
          return json({ title: "Not in the benchmark" }, 404);
      }
    },
  };
}
