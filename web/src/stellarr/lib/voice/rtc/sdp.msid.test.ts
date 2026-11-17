import { describe, expect, it } from "vitest";
import {
  OPUS_FMTP_PARAMETERS,
  OPUS_PTIME,
  tuneAudioSections,
  tuneLocalDescription,
} from "./sdp";

function sdpOf(...lines: string[]): string {
  return [...lines, ""].join("\r\n");
}

describe("tuneLocalDescription without an sdp", () => {
  it("hands a rollback description back untouched", () => {
    const rollback: RTCSessionDescriptionInit = { type: "rollback" };
    expect(tuneLocalDescription(rollback)).toBe(rollback);
  });
});

describe("tuneAudioSections with unusual rtpmap spacing", () => {
  it("never places a created fmtp line above the m-line", () => {
    const sdp = sdpOf(
      "v=0",
      "t=0 0",
      "m=audio 9 UDP/TLS/RTP/SAVPF 111",
      "a=rtpmap:111\topus/48000/2",
    );

    const lines = tuneAudioSections(sdp, () => ({
      parameters: OPUS_FMTP_PARAMETERS,
      ptime: OPUS_PTIME,
    })).split("\r\n");

    expect(lines.indexOf("m=audio 9 UDP/TLS/RTP/SAVPF 111")).toBe(2);
    expect(lines.slice(0, 2)).toEqual(["v=0", "t=0 0"]);
  });
});
