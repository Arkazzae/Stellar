import { afterEach, describe, expect, it, vi } from "vitest";

import { withoutHeaderExtension } from "../rtc/sdp";
import { AUDIO_LEVEL_EXTENSION, encryptableCodecs, restrictForEncryption } from "./encryptedMedia";

const codec = (mimeType: string, sdpFmtpLine?: string): RTCRtpCodec =>
  ({ mimeType, clockRate: 90_000, ...(sdpFmtpLine === undefined ? {} : { sdpFmtpLine }) }) as RTCRtpCodec;

const VIDEO = [
  codec("video/VP8"),
  codec("video/rtx"),
  codec("video/VP9", "profile-id=0"),
  codec("video/VP9", "profile-id=2"),
  codec("video/H264", "profile-level-id=42e01f"),
  codec("video/AV1"),
  codec("video/red"),
  codec("video/ulpfec"),
];
const AUDIO = [codec("audio/opus"), codec("audio/red"), codec("audio/CN"), codec("audio/telephone-event")];

const names = (codecs: RTCRtpCodec[] | null) => codecs?.map((item) => item.mimeType) ?? null;

describe("encryptableCodecs", () => {
  it("keeps Opus alone for a voice, and VP8 and VP9 with their retransmission and redundancy for a picture", () => {
    expect(names(encryptableCodecs(AUDIO, "audio"))).toEqual(["audio/opus"]);
    expect(names(encryptableCodecs(VIDEO, "video"))).toEqual([
      "video/VP8",
      "video/VP9",
      "video/VP9",
      "video/rtx",
      "video/red",
      "video/ulpfec",
    ]);
  });

  it("puts the screen's VP9 first and offers nothing when none of the three is there", () => {
    expect(names(encryptableCodecs(VIDEO, "video", "video/VP9"))?.slice(0, 3)).toEqual([
      "video/VP9",
      "video/VP9",
      "video/VP8",
    ]);
    expect(encryptableCodecs([codec("video/H264"), codec("video/rtx")], "video")).toBeNull();
  });
});

describe("restrictForEncryption", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("restricts every section, sending or receiving, and stops negotiating audio levels", () => {
    vi.stubGlobal("RTCRtpReceiver", {
      getCapabilities: (kind: string) => ({ codecs: kind === "audio" ? AUDIO : VIDEO }),
    });
    const transceiver = (kind: "audio" | "video", sender = {}) => ({
      sender,
      receiver: { track: { kind } },
      direction: "sendonly",
      currentDirection: null,
      setCodecPreferences: vi.fn(),
      getHeaderExtensionsToNegotiate: vi.fn(() => [
        { uri: AUDIO_LEVEL_EXTENSION, direction: "sendrecv" },
        { uri: "urn:ietf:params:rtp-hdrext:sdes:mid", direction: "sendrecv" },
      ]),
      setHeaderExtensionsToNegotiate: vi.fn(),
    });
    const screen = {} as RTCRtpSender;
    const mic = transceiver("audio");
    const shared = transceiver("video", screen);
    const received = { ...transceiver("video"), direction: "recvonly" };
    const connection = { getTransceivers: () => [mic, shared, received] } as unknown as RTCPeerConnection;

    restrictForEncryption(connection, screen);

    expect(names(mic.setCodecPreferences.mock.calls[0]![0])).toEqual(["audio/opus"]);
    expect(names(shared.setCodecPreferences.mock.calls[0]![0])?.[0]).toBe("video/VP9");
    expect(names(received.setCodecPreferences.mock.calls[0]![0])).not.toContain("video/H264");
    expect(mic.setHeaderExtensionsToNegotiate).toHaveBeenCalledWith([
      { uri: AUDIO_LEVEL_EXTENSION, direction: "stopped" },
      { uri: "urn:ietf:params:rtp-hdrext:sdes:mid", direction: "sendrecv" },
    ]);
    expect(shared.setHeaderExtensionsToNegotiate).not.toHaveBeenCalled();
  });
});

describe("withoutHeaderExtension", () => {
  it("drops the extension from every section of the kind and nothing else", () => {
    const sdp = [
      "v=0",
      "m=audio 9 UDP/TLS/RTP/SAVPF 111",
      "a=mid:0",
      `a=extmap:1 ${AUDIO_LEVEL_EXTENSION}`,
      "a=extmap:4 urn:ietf:params:rtp-hdrext:sdes:mid",
      "m=video 9 UDP/TLS/RTP/SAVPF 96",
      "a=mid:1",
      `a=extmap:1 ${AUDIO_LEVEL_EXTENSION}`,
      "m=audio 9 UDP/TLS/RTP/SAVPF 111",
      "a=mid:2",
      `a=extmap:1/sendonly ${AUDIO_LEVEL_EXTENSION}`,
      "",
    ].join("\r\n");
    const out = withoutHeaderExtension(sdp, AUDIO_LEVEL_EXTENSION, "audio");
    expect(out.split("\r\n").filter((line) => line.includes(AUDIO_LEVEL_EXTENSION))).toEqual([
      `a=extmap:1 ${AUDIO_LEVEL_EXTENSION}`,
    ]);
    expect(out).toContain("a=extmap:4 urn:ietf:params:rtp-hdrext:sdes:mid");
    expect(withoutHeaderExtension("v=0\r\n", AUDIO_LEVEL_EXTENSION, "audio")).toBe("v=0\r\n");
  });
});
