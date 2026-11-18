/**
 * What an encrypted call negotiates, both ways.
 *
 * Only Opus, VP8 and VP9 survive encryption frame by frame: the browser's
 * packetizers for H.264 and AV1 read the frame they are given, so a sealed
 * one cannot be sent at all. Every section of an encrypted call's connection
 * therefore offers and accepts those three only, with the retransmission and
 * redundancy entries that go with a picture.
 *
 * The voice also carries less about itself. The audio-level header extension
 * would tell the media server how loud every packet is, in the clear, so it is
 * not negotiated; and Opus is asked for a constant bitrate without
 * discontinuous transmission, so the sizes and the gaps of packets do not
 * follow speech.
 */

const PRIMARY: Record<"audio" | "video", ReadonlySet<string>> = {
  audio: new Set(["audio/opus"]),
  video: new Set(["video/vp8", "video/vp9"]),
};

/** Entries that are not codecs of their own: retransmission and redundancy for a picture. */
const COMPANIONS = new Set(["video/rtx", "video/red", "video/ulpfec", "video/flexfec-03"]);

export const AUDIO_LEVEL_EXTENSION = "urn:ietf:params:rtp-hdrext:ssrc-audio-level";

/** Opus parameters laid over every voice section of an encrypted call. */
export const ENCRYPTED_OPUS_PARAMETERS: Readonly<Record<string, string>> = Object.freeze({
  cbr: "1",
  usedtx: "0",
});

/**
 * The codecs an encrypted section may use, in the browser's order with
 * `first` moved to the front; null when the browser has none of them.
 */
export function encryptableCodecs(
  codecs: readonly RTCRtpCodec[] | undefined,
  kind: "audio" | "video",
  first?: string,
): RTCRtpCodec[] | null {
  if (!codecs) return null;
  const mime = (codec: RTCRtpCodec) => codec.mimeType.toLowerCase();
  const primary = codecs.filter((codec) => PRIMARY[kind].has(mime(codec)));
  if (primary.length === 0) return null;
  const wanted = first?.toLowerCase();
  const ordered = [
    ...primary.filter((codec) => mime(codec) === wanted),
    ...primary.filter((codec) => mime(codec) !== wanted),
  ];
  return [...ordered, ...codecs.filter((codec) => COMPANIONS.has(mime(codec)))];
}

type ExtensionCapable = RTCRtpTransceiver & {
  getHeaderExtensionsToNegotiate?: () => { uri: string; direction: RTCRtpTransceiverDirection | "stopped" }[];
  setHeaderExtensionsToNegotiate?: (
    extensions: { uri: string; direction: RTCRtpTransceiverDirection | "stopped" }[],
  ) => void;
};

/**
 * Restricts every section of an encrypted call's connection before a
 * description is made: the codecs above, the screen's picture VP9 first, and
 * no audio levels where the browser lets that be said. The SDP is edited as
 * well (`withoutHeaderExtension`), for browsers that do not.
 */
export function restrictForEncryption(connection: RTCPeerConnection, screenVideo: RTCRtpSender | null): void {
  const capabilities = (kind: "audio" | "video") =>
    typeof RTCRtpReceiver !== "undefined" && RTCRtpReceiver.getCapabilities
      ? RTCRtpReceiver.getCapabilities(kind)?.codecs
      : undefined;
  for (const transceiver of connection.getTransceivers()) {
    if (transceiver.currentDirection === "stopped" || transceiver.direction === "stopped") continue;
    const kind = transceiver.receiver.track.kind === "video" ? "video" : "audio";
    const codecs = encryptableCodecs(
      capabilities(kind),
      kind,
      transceiver.sender === screenVideo ? "video/VP9" : undefined,
    );
    try {
      if (codecs !== null && typeof transceiver.setCodecPreferences === "function") {
        transceiver.setCodecPreferences(codecs);
      }
    } catch {
      // A stack that refuses the list still has the frame worker, which
      // sends no picture of a codec it cannot encrypt.
    }
    if (kind !== "audio") continue;
    const capable = transceiver as ExtensionCapable;
    try {
      const extensions = capable.getHeaderExtensionsToNegotiate?.();
      if (extensions && capable.setHeaderExtensionsToNegotiate) {
        capable.setHeaderExtensionsToNegotiate(
          extensions.map((extension) =>
            extension.uri === AUDIO_LEVEL_EXTENSION ? { ...extension, direction: "stopped" } : extension,
          ),
        );
      }
    } catch {
      // The SDP edit covers it.
    }
  }
}
