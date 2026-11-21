import type { ReactNode } from "react";

import { Icon, type IconName } from "@stellarr/icons";
import { RoleMark } from "@/components/settings/server/panes/RoleMark";
import { ServerMark } from "@/components/settings/server/panes/ServerMark";

import { Avatar } from "./Avatar";

type ChannelKind = "text" | "voice";

/**
 * The one thing a question is about, described as data so the requests in
 * `confirmations.ts` stay plain functions. The card draws each kind with the
 * component the app draws it with elsewhere, so the row is recognised rather
 * than read.
 */
export type ConfirmSubject =
  | {
      kind: "person";
      id: string;
      name: string;
      avatarUrl?: string;
      /** The username, without the "@"; left out when it is not at hand. */
      handle?: string;
    }
  | { kind: "channel"; channelKind: ChannelKind; name: string; detail: string }
  | { kind: "server"; name: string; mark: string; iconUrl?: string; detail?: string }
  | { kind: "message"; authorId: string; authorName: string; authorAvatarUrl?: string; text: string }
  | {
      kind: "section";
      name: string;
      channels: readonly { id: string; name: string; channelKind: ChannelKind }[];
      /** Said under the caption when the section holds no channels. */
      emptyDetail?: string;
    }
  | { kind: "role"; name: string; color: string; isOwner?: boolean; detail?: string }
  | {
      kind: "item";
      name: string;
      detail?: string;
      /** An icon, a picture (a custom emoji) or a character (a sound's glyph). */
      mark: { icon: IconName } | { image: string } | { glyph: string };
    };

function Row({ lead, name, children }: { lead?: ReactNode; name: string; children?: ReactNode }) {
  return (
    <div className="cf-subject">
      {lead === undefined ? null : <span className="cf-subject-lead">{lead}</span>}
      <span className="cf-subject-txt">
        <b>{name}</b>
        {children}
      </span>
    </div>
  );
}

function Detail({ text }: { text: string | undefined }) {
  return text === undefined || text === "" ? null : <span className="cf-subject-detail">{text}</span>;
}

function Tile({ children }: { children: ReactNode }) {
  return (
    <span className="cf-subject-tile" aria-hidden="true">
      {children}
    </span>
  );
}

const channelGlyph = (kind: ChannelKind) => (kind === "voice" ? <Icon name="speaker" size={16} /> : "#");

/** The subject row under the question's sentence. */
export function ConfirmSubjectRow({ subject }: { subject: ConfirmSubject }) {
  switch (subject.kind) {
    case "person":
      return (
        <Row
          lead={<Avatar id={subject.id} displayName={subject.name} avatarUrl={subject.avatarUrl} size="md" />}
          name={subject.name}
        >
          <Detail text={subject.handle === undefined ? undefined : `@${subject.handle}`} />
        </Row>
      );
    case "channel":
      return (
        <Row lead={<Tile>{channelGlyph(subject.channelKind)}</Tile>} name={subject.name}>
          <Detail text={subject.detail} />
        </Row>
      );
    case "server":
      return (
        <Row lead={<ServerMark mark={subject.mark} iconUrl={subject.iconUrl} />} name={subject.name}>
          <Detail text={subject.detail} />
        </Row>
      );
    case "message":
      return (
        <Row
          lead={
            <Avatar
              id={subject.authorId}
              displayName={subject.authorName}
              avatarUrl={subject.authorAvatarUrl}
              size="md"
            />
          }
          name={subject.authorName}
        >
          <span className="cf-subject-quote">{subject.text}</span>
        </Row>
      );
    case "section":
      // No leading slot: a section is a caption in the rail, not a picture.
      return (
        <div className="cf-subject cf-subject-section">
          <span className="cf-subject-txt">
            <span className="group-caption">{subject.name}</span>
            {subject.channels.length === 0 ? (
              <Detail text={subject.emptyDetail} />
            ) : (
              subject.channels.map((channel) => (
                <span className="cf-subject-channel" key={channel.id}>
                  <span className="cf-subject-glyph" aria-hidden="true">
                    {channelGlyph(channel.channelKind)}
                  </span>
                  {channel.name}
                </span>
              ))
            )}
          </span>
        </div>
      );
    case "role":
      return (
        <Row
          lead={
            <Tile>
              <RoleMark role={{ color: subject.color, isOwner: subject.isOwner === true }} size={18} />
            </Tile>
          }
          name={subject.name}
        >
          <Detail text={subject.detail} />
        </Row>
      );
    case "item": {
      const { mark } = subject;
      const lead =
        "image" in mark ? (
          <Tile>
            <img src={mark.image} alt="" width={26} height={26} />
          </Tile>
        ) : "glyph" in mark ? (
          <Tile>{mark.glyph}</Tile>
        ) : (
          <Tile>
            <Icon name={mark.icon} size={18} />
          </Tile>
        );
      return (
        <Row lead={lead} name={subject.name}>
          <Detail text={subject.detail} />
        </Row>
      );
    }
  }
}
