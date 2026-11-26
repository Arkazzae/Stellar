/**
 * View models: the shapes presentational components take.
 *
 * Components in `src/stellarr/components/**` are pure. They render these and raise
 * callbacks; containers in `src/stellarr/state/**` build them from the ports.
 */

import type { Presence } from "@/ports";

export interface PersonVM {
  id: string;
  displayName: string;
  /** Two letters, precomputed so the avatar never re-derives them. */
  initials: string;
  presence: Presence;
  /** A picture, when they have one. The initials stay underneath it. */
  avatarUrl?: string;
  /** Only meaningful for someone in the current call. */
  speaking?: boolean;
  muted?: boolean;
  /** Someone on a voice channel shares a screen, as the server lists them. */
  sharing?: boolean;
  /** Someone on a voice channel has their camera on, as the server lists them. */
  camera?: boolean;
  /** Overrides the id-derived colour when the server supplies one. */
  color?: string;
  /** In an encrypted call: their sound or picture arrives but does not decrypt. */
  undecryptable?: { audio: boolean; video: boolean };
}

/**
 * Raised by any control that names a person: a roster row, an avatar in the
 * transcript, a friend. The mini profile hangs off the clicked element, so the
 * element travels with the id.
 */
export type OpenProfile = (userId: string, anchor: HTMLElement) => void;

/** Where someone stands in relation to you; decides what the card offers. */
export type ProfileRelation =
  | "self"
  | "friend"
  | "blocked"
  | "incoming"
  | "outgoing"
  | "none";

export interface ProfileRoleVM {
  id: string;
  name: string;
  /** Any CSS colour: roles are data, not theme tokens. */
  color: string;
}

/** A person as the mini profile shows them. */
export interface ProfileVM {
  person: PersonVM;
  /** Without the "@"; the card prints it under the display name. */
  username: string;
  relation: ProfileRelation;
  /** Their own artwork. Absent falls back to the id-derived hue. */
  bannerUrl?: string;
  /** A short, current line, separate from the longer bio. */
  customStatus?: string;
  /** What they wrote about themselves. Plain text; the card never parses it. */
  bio?: string;
  /** "ona/jej", printed beside the username exactly as they typed it. */
  pronouns?: string;
  /** The voice channel they are in, if any. */
  voiceChannelName?: string;
  roles: ProfileRoleVM[];
  /** "12 lipca 2026": the day they joined the server. */
  joinedAt?: string;
  /** "3 marca 2026": the day the friendship began. */
  friendsSince?: string;
  /** Answering or withdrawing a pending invitation needs its id. */
  requestId?: string;
}

export interface TextChannelVM {
  id: string;
  name: string;
  topic?: string;
  unread?: number;
  muted?: boolean;
  /** The rail section this channel is grouped under; absent if unsectioned. */
  sectionId?: string;
}

/** A collapsible rail group, in display order. */
export interface SectionVM {
  id: string;
  name: string;
}

/** A channel dropped in the rail, in its own group or another one. */
export interface ChannelMove {
  channelId: string;
  /** The section of the group it landed in; null outside sections. */
  sectionId: string | null;
  /** That group's complete id list in its new order, the moved channel included. */
  order: string[];
}

/** A compact thread row nested below its text channel in the rail. */
export interface ChannelThreadVM {
  id: string;
  channelId: string;
  authorName: string;
  preview: string;
  replyCount: number;
}

export interface VoiceChannelVM {
  id: string;
  name: string;
  occupants: PersonVM[];
  /** The rail section this channel is grouped under; absent if unsectioned. */
  sectionId?: string;
  /** How many people the channel seats; absent when it seats anyone who comes. */
  userLimit?: number;
  /** The activity running on the channel, whose mark follows the names of the people inside it. */
  activity?: VoiceChannelActivityVM;
}

/** What runs on a voice channel, as the rail marks the people inside it. */
export interface VoiceChannelActivityVM {
  install: string;
  name: string;
  icon: string | null;
  /** Who is inside, by id. */
  participants: readonly string[];
}

/** A private thread as the rail lists it. */
export interface DmConversationVM {
  id: string;
  peer: PersonVM;
  /**
   * The second line, already composed ("Ada: uploaded", "You: will let you know", or the
   * invitation to start), so the rail never rebuilds a sentence from parts and
   * an empty thread is not a special case there.
   */
  preview: string;
  unread: number;
  /** Someone is occupying this conversation's private RTC room. */
  callActive?: boolean;
  /** End-to-end encryption is on: a lock before the name. */
  encrypted?: boolean;
  /** When the last message was sent, shortened ("2 min", "wczoraj"). Absent before the first. */
  time?: { text: string; iso: string };
}

/** Which list the Friends view shows. "add" is a panel, not a filter. */
export type FriendsTab = "online" | "all" | "pending" | "blocked" | "add";

/** A person as the Friends view lists them. */
export interface FriendVM {
  person: PersonVM;
  /** Without the "@"; the row prints it after the display name. */
  username: string;
  /** Formatted date the friendship began. Absent for a blocked person. */
  since?: string;
  /** What they say about themselves, trimmed; absent when they say nothing. */
  customStatus?: string;
}

/** A reachable friend and the activity visible to the signed-in user. */
export interface ActiveFriendVM {
  person: PersonVM;
  customStatus?: string;
  inCall: boolean;
  /** Only channels in the current server's visible directory are included. */
  voiceChannel?: { name: string; serverName: string };
}

export interface FriendRequestVM {
  id: string;
  person: PersonVM;
  username: string;
  /** Incoming is answered, outgoing is withdrawn; the row differs. */
  direction: "incoming" | "outgoing";
  /** "Sent 3 days ago". */
  when: string;
}

/** What a seat shows of someone in this client's call: the ring, the microphone, a screen and a camera. */
export interface VoiceSeatVM {
  readonly speaking: boolean;
  readonly muted: boolean;
  readonly sharing: boolean;
  readonly camera: boolean;
}

/**
 * Where a row reads one person's seat, and hears when it changes. Null for
 * anyone outside this client's call.
 */
export interface VoiceSeatSource {
  get(userId: string): VoiceSeatVM | null;
  subscribe(userId: string, listener: () => void): () => void;
}

/** Which pane the room shows. Voice replaces the transcript, never splits it. */
export type RoomView = "text" | "voice";

export interface CallVM {
  channelId: string;
  channelName: string;
  /** A direct-conversation RTC room rather than a public server channel. */
  privateCall?: boolean;
  /** This side encrypts the call end to end. */
  encrypted?: boolean;
  /** Epoch ms the call started, for the dock's elapsed counter. */
  startedAt: number;
  muted: boolean;
  deafened: boolean;
  cameraOn: boolean;
  cameraPending: boolean;
  sharing: boolean;
  /** Worst media RTT, or signalling RTT while no peer media exists. */
  latencyMs: number | null;
  /** Coarse quality derived from RTT and packet loss. */
  connectionQuality: "unknown" | "good" | "fair" | "poor";
  /** The dock's headline: live content wins over transport status. */
  headline:
    | "requesting-mic"
    | "connecting"
    | "connected"
    | "reconnecting"
    | "degraded"
    | "failed"
    | "screen";
}

/**
 * One screen shared in the call. Every share is its own tile on the stage; at
 * most one remote share is watched at a time, because the client asks the
 * media server for a single screen.
 */
export interface ScreenShareVM {
  /**
   * Names one concrete stream, so opting in to watch it never carries over to
   * the next stream or owner a reconnect brings.
   */
  key: string;
  ownerId: string;
  ownerName: string;
  /** This user's own screen: previewed muted, never watched or opted into. */
  local: boolean;
  hasAudio: boolean;
  /** Null while nobody here watches it, and while its picture is on its way. */
  stream: MediaStream | null;
  /** A still to show before the viewer opts in, when there is no stream. */
  posterUrl?: string;
  watching: boolean;
  /** Watched a moment ago and the first picture has not come yet. */
  connecting?: boolean;
  /** Watched, but the picture is held back: the viewer's link has no room for it now. */
  heldBack?: boolean;
  /** In an encrypted call: the picture reaches this side but does not decrypt. */
  sealed?: boolean;
}
