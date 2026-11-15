import { buildTranscript } from "@/lib/format";
import type { Message, MessageReaction } from "@/ports";
import {
  createTranscriptPresenter,
  presentTranscript,
  type PresentationContext,
} from "@/state/chat/present";

import type { TranscriptSectionVM } from "@/components/room/Transcript";

/** One message every 137 s: about 630 messages to a day, so day rows are real. */
export const STEP_MS = 137_000;
export const BASE_MS = Date.UTC(2026, 10, 15, 12, 0, 0);

const AUTHORS = [
  "Ada",
  "Maja",
  "Nils",
  "Zośka",
  "Kuba",
  "Iga",
  "Tomasz",
  "Lena",
  "Bartek",
  "Ola",
  "Rafał",
  "Ewa",
];

const imageUrls = new Map<string, string>();

/**
 * A deterministic picture whose intrinsic size is the measurement in the
 * attachment. The reserved box and the loaded picture therefore agree: the
 * benchmark measures the real component, not a box that collapses to the
 * 4×3 placeholder after `onLoad`.
 */
function imageUrl(width: number, height: number): string {
  const key = `${width}x${height}`;
  const cached = imageUrls.get(key);
  if (cached !== undefined) return cached;
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}">` +
    `<rect width="${width}" height="${height}" fill="#343a4d"/>` +
    `<circle cx="${Math.round(width / 2)}" cy="${Math.round(height / 2)}" r="${Math.round(Math.min(width, height) / 6)}" fill="#7d849c"/>` +
    `</svg>`;
  const url = `data:image/svg+xml,${encodeURIComponent(svg)}`;
  imageUrls.set(key, url);
  return url;
}

/** Deterministic, so two runs measure the same text and the same boxes. */
function random(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let value = state;
    value = Math.imul(value ^ (value >>> 15), value | 1);
    value ^= value + Math.imul(value ^ (value >>> 7), value | 61);
    return ((value ^ (value >>> 14)) >>> 0) / 4_294_967_296;
  };
}

function sentences(count: number, next: () => number): string {
  const words = [
    "rozmowa",
    "kanał",
    "zdjęcie",
    "spotkanie",
    "projekt",
    "wieczorem",
    "dobrze",
    "zobaczymy",
    "jeszcze",
    "właśnie",
    "krótko",
    "potem",
    "tutaj",
    "wszyscy",
    "może",
    "razem",
    "inaczej",
    "spokojnie",
  ];
  const lines: string[] = [];
  for (let index = 0; index < count; index += 1) {
    const length = 4 + Math.floor(next() * 9);
    const wordsInLine = Array.from(
      { length },
      () => words[Math.floor(next() * words.length)] ?? "razem",
    );
    lines.push(wordsInLine.join(" "));
  }
  return lines.join("\n");
}

function imageSize(index: number): { width: number; height: number } {
  const shapes = [
    { width: 1200, height: 800 },
    { width: 900, height: 1600 },
    { width: 640, height: 640 },
    { width: 1600, height: 600 },
  ];
  return shapes[index % shapes.length] ?? shapes[0]!;
}

/**
 * A run of messages at consecutive indexes. Indexes may be negative for the
 * pages that are prepended before the first one; the id keeps the sign so no
 * two pages can collide.
 */
export function makeMessages(startIndex: number, count: number, seed = 17): Message[] {
  const next = random(seed + startIndex);
  const messages: Message[] = [];
  for (let offset = 0; offset < count; offset += 1) {
    const index = startIndex + offset;
    const author = AUTHORS[Math.abs(index) % AUTHORS.length] ?? "Ada";
    const authorId = `u-${Math.abs(index) % AUTHORS.length}`;
    const createdAt = new Date(BASE_MS + index * STEP_MS).toISOString();
    const media = Math.abs(index) % 19 === 0;
    const size = imageSize(index);
    const long = Math.abs(index) % 7 === 0;
    const answered = Math.abs(index) % 5 === 0 && offset > 0;
    const reacted = Math.abs(index) % 11 === 0;
    const pinned = Math.abs(index) % 97 === 0;

    const text = [
      long ? sentences(12, next) : sentences(1 + (Math.abs(index) % 3), next),
      Math.abs(index) % 23 === 0 ? "Zerknij na https://example.test/roadmap?q=1 i @Maja." : "",
      Math.abs(index) % 31 === 0 ? "**ważne** *dzisiaj* `kod`" : "",
    ]
      .filter(Boolean)
      .join("\n");

    const reactions: MessageReaction[] = reacted
      ? [
          { emoji: "👍", count: 1 + (Math.abs(index) % 5), mine: false },
          { emoji: "🎉", count: 1, mine: true },
        ]
      : [];

    messages.push({
      id: `m-${index}`,
      channelId: "c-bench",
      author: { id: authorId, displayName: author },
      content: text,
      attachments: media
        ? [
            {
              id: `a-${index}`,
              url: imageUrl(size.width, size.height),
              name: `zdjęcie-${index}.svg`,
              contentType: "image/svg+xml",
              size: 120,
              ...size,
            },
          ]
        : [],
      embeds: [],
      createdAt,
      ...(answered
        ? {
            replyTo: {
              id: `m-${index - 1}`,
              authorId,
              authorName: author,
              preview: "poprzednia wiadomość",
            },
          }
        : {}),
      ...(pinned ? { pinnedAt: createdAt } : {}),
      ...(reactions.length > 0 ? { reactions } : {}),
    });
  }
  return messages;
}

/** Puts a single searchable phrase in one message, at a known row. */
export function markerFor(count: number): { index: number; text: string } {
  const start = Math.max(0, Math.floor(count * 0.7));
  // Not a long (collapsed), media or reply row: the marker must be ordinary
  // visible text when native find reaches it. A tiny history may not have such
  // a row; the last one is still better than no marker at all.
  for (let offset = 0; offset < Math.max(1, count); offset += 1) {
    const index = (start + offset) % Math.max(1, count);
    if (index % 7 !== 0 && index % 19 !== 0 && index % 5 !== 0) {
      return { index, text: `needle-${count}-${index}` };
    }
  }
  const fallback = Math.max(0, count - 1);
  return { index: fallback, text: `needle-${count}-${fallback}` };
}

/** Injects the marker into the generated body without changing row count. */
export function withMarker(messages: Message[], count: number): Message[] {
  const marker = markerFor(count);
  return messages.map((message) =>
    message.id === `m-${marker.index}`
      ? { ...message, content: `${message.content}\n${marker.text}` }
      : message,
  );
}

/** The producer chain the real providers run: messages to sections to props. */
function presentationContext(
  pinnedIds: ReadonlySet<string>,
  reactions: ReadonlyMap<string, MessageReaction[]>,
): PresentationContext {
  const faces = new Map(AUTHORS.map((_, index) => [`u-${index}`, `https://faces.test/${index}.png`]));
  return {
    selfId: "u-0",
    server: { name: "Bench", iconUrl: undefined },
    faces,
    customEmoji: new Map([[":party:", "https://emoji.test/party.png"]]),
    mentions: new Map([
      [
        "maja",
        {
          userId: "u-1",
          username: "Maja",
          displayName: "Maja",
          self: false,
        },
      ],
    ]),
    pinnedIds,
    reactionsOf: (messageId) => reactions.get(messageId) ?? [],
    extFallback: "FILE",
  };
}

/**
 * A stateful producer, matching what ChatProvider and FriendsProvider do: one
 * presenter per mounted conversation, and the same presentation context while
 * only the message list grows. The stateless `sectionsOf` below is kept for
 * one-off calls and for the recorded baseline.
 */
export function createSectionsProducer(): (messages: readonly Message[]) => TranscriptSectionVM[] {
  const present = createTranscriptPresenter();
  const reactions = new Map<string, MessageReaction[]>();
  const pinnedIds = new Set<string>();
  const context = presentationContext(pinnedIds, reactions);
  return (messages) => {
    for (const message of messages) {
      if (message.reactions !== undefined && message.reactions.length > 0) {
        reactions.set(message.id, message.reactions);
      }
      if (message.pinnedAt !== undefined) pinnedIds.add(message.id);
    }
    return present(buildTranscript([...messages], new Date(BASE_MS), "en"), context);
  };
}

/** The producer chain with no cache, for a single presentation. */
export function sectionsOf(messages: readonly Message[]): TranscriptSectionVM[] {
  const pinnedIds = new Set(
    messages.filter((message) => message.pinnedAt !== undefined).map((message) => message.id),
  );
  const reactions = new Map(
    messages
      .filter((message) => (message.reactions?.length ?? 0) > 0)
      .map((message) => [message.id, message.reactions ?? []]),
  );
  return presentTranscript(
    buildTranscript([...messages], new Date(BASE_MS), "en"),
    presentationContext(pinnedIds, reactions),
  );
}

/** Group/body counts that the DOM must contain, for the correctness checks. */
export function expectedRows(sections: readonly TranscriptSectionVM[]) {
  let groups = 0;
  let bodies = 0;
  for (const section of sections) {
    groups += section.groups.length;
    for (const group of section.groups) bodies += group.bodies.length;
  }
  return { rows: sections.length + groups, groups, bodies };
}
