import { describe, expect, it } from "vitest";
import {
  decodeBrowserAnswer,
  decodeBrowserOffer,
  decodeMediaCandidate,
  decodeMediaServerAnswer,
  decodeMediaServerOffer,
  decodeMediaTrack,
  decodeMediaTracks,
  decodeMediaSubscriptions,
  decodeRtcCandidate,
  decodeRtcDescription,
  decodeRtcState,
  encodeBrowserAnswer,
  encodeBrowserOffer,
  encodeMediaCandidate,
  encodeMediaServerAnswer,
  encodeMediaServerOffer,
  encodeMediaTrack,
  encodeMediaTracks,
  encodeMediaSubscriptions,
  encodeRtcCandidate,
  encodeRtcDescription,
  encodeRtcState,
  isBrowserMediaSignalKind,
  isMediaServerSignalKind,
  isMediaSessionCloseReason,
  isMediaSignalKind,
  isRtcSignalKind,
  RTC_HUB,
  RTC_PROTOCOL_VERSION,
  type MediaTrack,
} from "./protocol";

describe("Stellarr RTC protocol v1", () => {
  it("has one explicit wire version and lets browsers tell each other only their state", () => {
    expect(RTC_PROTOCOL_VERSION).toBe(1);
    expect(isRtcSignalKind("state")).toBe(true);
    // Descriptions and candidates go to the media server, never to another browser.
    for (const kind of ["offer", "answer", "candidate", "renegotiate"]) {
      expect(isRtcSignalKind(kind)).toBe(false);
    }
  });

  it("round-trips descriptions without accepting a mismatched kind", () => {
    const payload = encodeRtcDescription("offer", "v=0\r\n");
    expect(decodeRtcDescription(payload, "offer")).toEqual({
      type: "offer",
      sdp: "v=0\r\n",
    });
    expect(decodeRtcDescription(payload, "answer")).toBeNull();
  });

  it("distinguishes end-of-candidates from a malformed candidate", () => {
    expect(decodeRtcCandidate(encodeRtcCandidate(null))).toBeNull();
    expect(
      decodeRtcCandidate(
        encodeRtcCandidate({ candidate: "candidate:1", sdpMid: "0" }),
      ),
    ).toEqual({ candidate: "candidate:1", sdpMid: "0" });
    expect(decodeRtcCandidate('{"candidate":42}')).toBeUndefined();
    expect(decodeRtcCandidate("not-json")).toBeUndefined();
  });

  it("requires the complete participant-state payload", () => {
    const state = {
      muted: true,
      screenStreamId: "screen-1",
      cameraStreamId: "camera-1",
      encrypted: true,
    };
    expect(decodeRtcState(encodeRtcState(state))).toEqual(state);
    expect(decodeRtcState('{"muted":true}')).toBeNull();
    expect(decodeRtcState('{"muted":"yes","screenStreamId":null}')).toBeNull();
    // A side that does not say it encrypts does not.
    expect(decodeRtcState('{"muted":false,"screenStreamId":null}')).toMatchObject({ encrypted: false });
    expect(decodeRtcState('{"muted":false,"screenStreamId":null,"encrypted":"yes"}')).toBeNull();
  });

  it("accepts cameraStreamId as an optional RTC v1 extension", () => {
    expect(
      decodeRtcState('{"muted":false,"screenStreamId":null}'),
    ).toEqual({ muted: false, screenStreamId: null, cameraStreamId: null, encrypted: false });
    expect(
      decodeRtcState(
        '{"muted":false,"screenStreamId":null,"cameraStreamId":42}',
      ),
    ).toBeNull();
  });
});

describe("watching in the participant state", () => {
  it("carries the screen a side watches, only while it watches one", () => {
    const state = {
      muted: false,
      screenStreamId: null,
      cameraStreamId: null,
      encrypted: false,
      watching: { userId: "u-ola", streamId: "share-ola" },
    };
    expect(decodeRtcState(encodeRtcState(state))).toEqual(state);
    expect(
      encodeRtcState({ muted: false, screenStreamId: null, cameraStreamId: null, encrypted: false }),
    ).not.toContain("watching");
  });

  it("drops a malformed watching rather than the whole state", () => {
    expect(
      decodeRtcState('{"muted":true,"screenStreamId":null,"watching":{"userId":7}}'),
    ).toEqual({ muted: true, screenStreamId: null, cameraStreamId: null, encrypted: false });
  });
});

describe("Stellarr Media hub leg", () => {
  const mic: MediaTrack = { mid: "4", userId: "u-ola", kind: "audio", streamId: "mic-ola", heldBack: false };
  const share: MediaTrack = { mid: "5", userId: "u-ola", kind: "video", streamId: "share-ola", heldBack: true };

  it("names the two methods and two events of the media server's leg", () => {
    expect(RTC_HUB.methods.openMediaSession).toBe("OpenMediaSession");
    expect(RTC_HUB.methods.mediaSignal).toBe("SendMediaSignal");
    expect(RTC_HUB.events.mediaSignal).toBe("MediaSignal");
    expect(RTC_HUB.events.mediaSessionClosed).toBe("MediaSessionClosed");
  });

  it("has a closed kind set, split by who may send which kind", () => {
    for (const kind of ["offer", "answer", "candidate", "tracks", "subscriptions"]) {
      expect(isMediaSignalKind(kind)).toBe(true);
    }
    expect(isMediaSignalKind("state")).toBe(false);
    expect(isBrowserMediaSignalKind("candidate")).toBe(true);
    expect(isBrowserMediaSignalKind("subscriptions")).toBe(true);
    expect(isBrowserMediaSignalKind("tracks")).toBe(false);
    expect(isMediaServerSignalKind("tracks")).toBe(true);
    expect(isMediaServerSignalKind("candidate")).toBe(false);
    expect(isMediaServerSignalKind("subscriptions")).toBe(false);
  });

  it("knows exactly the three reasons a media session is closed for", () => {
    for (const reason of ["media-restarted", "transport-timeout", "error"]) {
      expect(isMediaSessionCloseReason(reason)).toBe(true);
    }
    expect(isMediaSessionCloseReason("left")).toBe(false);
  });

  it("round-trips the browser's offer and the media server's answer", () => {
    expect(JSON.parse(encodeBrowserOffer("v=0\r\n"))).toEqual({ type: "offer", sdp: "v=0\r\n" });
    expect(decodeBrowserOffer(encodeBrowserOffer("v=0\r\n"))).toEqual({
      type: "offer",
      sdp: "v=0\r\n",
    });
    expect(JSON.parse(encodeMediaServerAnswer("v=0\r\n"))).toEqual({
      type: "answer",
      sdp: "v=0\r\n",
    });
    expect(decodeMediaServerAnswer(encodeMediaServerAnswer("v=0\r\n"))).toEqual({
      type: "answer",
      sdp: "v=0\r\n",
    });
    expect(decodeMediaServerAnswer(encodeBrowserOffer("v=0\r\n"))).toBeNull();
    expect(decodeBrowserOffer('{"type":"offer"}')).toBeNull();
  });

  it("round-trips the media server's offer with its number and directory", () => {
    const payload = encodeMediaServerOffer({ sdp: "v=0\r\n", negotiationId: 7, tracks: [mic, share] });

    expect(JSON.parse(payload)).toEqual({
      type: "offer",
      sdp: "v=0\r\n",
      negotiationId: 7,
      tracks: [mic, share],
    });
    expect(decodeMediaServerOffer(payload)).toEqual({
      type: "offer",
      sdp: "v=0\r\n",
      negotiationId: 7,
      tracks: [mic, share],
    });
  });

  it("reads an offer exactly as Stellarr Media writes it and the hub relays it", () => {
    // The shape the release binary put on the wire through AppHub, with a stub
    // in place of its SDP: the keys in the order serde_json writes them,
    // str0m's three-character mids, the stream ids of the publisher's msid and
    // its account in lower case.
    const payload =
      '{"negotiationId":1,"sdp":"v=0\\r\\no=- 1 2 IN IP4 0.0.0.0\\r\\n","tracks":[' +
      '{"heldBack":false,"kind":"audio","mid":"00D","streamId":"acca9081-8bfa-4421-beaf-13927c77972c",' +
      '"userId":"30000000-0000-0000-0000-000000000001"},' +
      '{"heldBack":true,"kind":"video","mid":"hnB","streamId":"71aba268-e9bb-43a0-b69e-d98e512505c9",' +
      '"userId":"30000000-0000-0000-0000-000000000001"}],"type":"offer"}';

    expect(decodeMediaServerOffer(payload)).toEqual({
      type: "offer",
      sdp: "v=0\r\no=- 1 2 IN IP4 0.0.0.0\r\n",
      negotiationId: 1,
      tracks: [
        {
          mid: "00D",
          userId: "30000000-0000-0000-0000-000000000001",
          kind: "audio",
          streamId: "acca9081-8bfa-4421-beaf-13927c77972c",
          heldBack: false,
        },
        {
          mid: "hnB",
          userId: "30000000-0000-0000-0000-000000000001",
          kind: "video",
          streamId: "71aba268-e9bb-43a0-b69e-d98e512505c9",
          heldBack: true,
        },
      ],
    });
  });

  it("rejects a media server offer without a usable number or directory", () => {
    const offer = { type: "offer", sdp: "v=0\r\n", negotiationId: 7, tracks: [mic] };
    for (const broken of [
      { ...offer, negotiationId: undefined },
      { ...offer, negotiationId: "7" },
      { ...offer, negotiationId: -1 },
      { ...offer, negotiationId: 1.5 },
      { ...offer, tracks: undefined },
      { ...offer, tracks: [mic, { ...share, kind: "screen" }] },
      { ...offer, type: "answer" },
      { ...offer, sdp: 42 },
    ]) {
      expect(decodeMediaServerOffer(JSON.stringify(broken))).toBeNull();
    }
    expect(decodeMediaServerOffer("not-json")).toBeNull();
    // A browser offer is not a media server offer: it carries neither.
    expect(decodeMediaServerOffer(encodeBrowserOffer("v=0\r\n"))).toBeNull();
  });

  it("round-trips the browser's answer and requires the offer's number in it", () => {
    const payload = encodeBrowserAnswer("v=0\r\n", 7);

    expect(JSON.parse(payload)).toEqual({ type: "answer", sdp: "v=0\r\n", negotiationId: 7 });
    expect(decodeBrowserAnswer(payload)).toEqual({
      type: "answer",
      sdp: "v=0\r\n",
      negotiationId: 7,
    });
    expect(decodeBrowserAnswer(encodeMediaServerAnswer("v=0\r\n"))).toBeNull();
    expect(decodeBrowserAnswer('{"type":"offer","sdp":"v=0","negotiationId":7}')).toBeNull();
  });

  it("carries an active TCP candidate and the end-of-candidates marker", () => {
    const candidate = {
      candidate: "candidate:1 1 tcp 1518280447 192.0.2.10 9 typ host tcptype active",
      sdpMid: "0",
      sdpMLineIndex: 0,
      usernameFragment: "offer",
    };

    expect(decodeMediaCandidate(encodeMediaCandidate(candidate))).toEqual(candidate);
    expect(encodeMediaCandidate(null)).toBe("null");
    expect(decodeMediaCandidate(encodeMediaCandidate(null))).toBeNull();
    expect(decodeMediaCandidate('{"candidate":42}')).toBeUndefined();
  });

  it("round-trips the whole directory and keeps only the contract's fields", () => {
    const payload = encodeMediaTracks([{ ...mic, extra: true } as MediaTrack, share]);

    expect(JSON.parse(payload)).toEqual({ tracks: [mic, share] });
    expect(decodeMediaTracks(payload)).toEqual([mic, share]);
    expect(decodeMediaTracks('{"tracks":[]}')).toEqual([]);
  });

  it("round-trips a complete subscriptions allowlist, including an empty one", () => {
    expect(encodeMediaSubscriptions(["00D", "screen-2"])).toBe('{"mids":["00D","screen-2"]}');
    expect(decodeMediaSubscriptions(encodeMediaSubscriptions(["00D", "screen-2"]))).toEqual({
      mids: ["00D", "screen-2"],
    });
    expect(decodeMediaSubscriptions(encodeMediaSubscriptions([]))).toEqual({ mids: [] });
    expect(decodeMediaSubscriptions(encodeMediaSubscriptions(["mic", "cam"], ["cam"]))).toEqual({
      mids: ["mic", "cam"], thumbnailMids: ["cam"],
    });
    for (const invalid of [["outside"], ["cam", "cam"]]) {
      expect(() => encodeMediaSubscriptions(["mic", "cam"], invalid)).toThrow(TypeError);
    }
    const limit = Array.from({ length: 4096 }, (_, index) => String(index));
    expect(decodeMediaSubscriptions(encodeMediaSubscriptions(limit))?.mids).toHaveLength(4096);
    expect(decodeMediaSubscriptions(encodeMediaSubscriptions(["x".repeat(16)]))).toEqual({
      mids: ["x".repeat(16)],
    });
  });

  it("rejects malformed subscriptions and refuses to encode an invalid allowlist", () => {
    for (const mids of [
      ["a", "a"], [""], ["a b"], ["a\tb"], ["a\nb"], ["a\u0000b"], ["\u007f"],
      ["ż"], ["x".repeat(17)], [42], [null], Array.from({ length: 4097 }, (_, index) => String(index)),
    ]) {
      expect(decodeMediaSubscriptions(JSON.stringify({ mids }))).toBeNull();
      expect(() => encodeMediaSubscriptions(mids as string[])).toThrow(TypeError);
    }
    for (const payload of ["not-json", "null", "[]", "{}", '{"mids":"a"}']) {
      expect(decodeMediaSubscriptions(payload)).toBeNull();
    }
  });

  it("rejects the whole directory when one entry is malformed", () => {
    for (const entry of [
      { ...mic, mid: "" },
      { ...mic, userId: 7 },
      { ...mic, kind: "data" },
      { ...mic, streamId: null },
      { ...mic, heldBack: undefined },
      { ...mic, heldBack: "false" },
      { ...mic, heldBack: 0 },
      "4",
    ]) {
      expect(decodeMediaTracks(JSON.stringify({ tracks: [share, entry] }))).toBeNull();
    }
    expect(decodeMediaTracks('{"tracks":{}}')).toBeNull();
    expect(decodeMediaTracks("[]")).toBeNull();
    expect(decodeMediaTracks("not-json")).toBeNull();
  });

  it("rejects a directory that names one section twice", () => {
    expect(decodeMediaTracks(encodeMediaTracks([mic, { ...share, mid: mic.mid }]))).toBeNull();
  });

  it("writes one directory entry with the contract's fields only", () => {
    const entry = encodeMediaTrack({ ...share, codec: "vp9" } as MediaTrack);

    expect(entry).toEqual(share);
    expect(decodeMediaTrack(JSON.parse(JSON.stringify(entry)))).toEqual(share);
  });

  it("reads one directory entry and ignores fields it does not know", () => {
    expect(decodeMediaTrack({ ...mic, codec: "opus" })).toEqual(mic);
    expect(decodeMediaTrack({ ...mic, streamId: "" })).toEqual({ ...mic, streamId: "" });
    expect(decodeMediaTrack(null)).toBeNull();
  });

  it("says of every entry whether it is held back, and requires it", () => {
    const entry = { mid: "5", userId: "u-ola", kind: "video", streamId: "share-ola" };
    expect(decodeMediaTrack({ ...entry, heldBack: true })).toEqual({ ...entry, heldBack: true });
    expect(decodeMediaTrack({ ...entry, heldBack: false })).toEqual({ ...entry, heldBack: false });
    expect(decodeMediaTrack(entry)).toBeNull();
    expect(JSON.parse(encodeMediaTracks([share])).tracks[0].heldBack).toBe(true);
  });
});
