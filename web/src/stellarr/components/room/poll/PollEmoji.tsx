import type { CustomEmoji } from "@/ports";

const bare = (shortcode: string) => shortcode.replace(/^:|:$/g, "");

export interface PollEmojiProps {
  /** As a reaction names it: the character, or `:shortcode:` for a server emoji. */
  emoji: string | undefined;
  customEmoji: readonly CustomEmoji[];
  size?: number;
}

/**
 * An answer's emoji, drawn the way a reaction draws its own: the server's
 * image for one of its emoji, the character for any other. A server emoji
 * that is gone draws nothing rather than its code.
 */
export function PollEmoji({ emoji, customEmoji, size = 18 }: PollEmojiProps) {
  if (emoji === undefined) return null;
  if (!emoji.startsWith(":")) {
    return (
      <span className="emo" aria-hidden="true" style={{ fontSize: size - 2, lineHeight: 1 }}>
        {emoji}
      </span>
    );
  }
  const image = customEmoji.find((item) => bare(item.shortcode) === bare(emoji));
  if (image === undefined) return null;
  return <img src={image.glyph} alt="" width={size} height={size} style={{ flex: "none", objectFit: "contain" }} />;
}
