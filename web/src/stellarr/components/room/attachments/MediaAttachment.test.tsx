import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { Attachment } from "./Attachment";
import { AudioAttachment } from "./AudioAttachment";
import { VideoAttachment } from "./VideoAttachment";

afterEach(() => vi.restoreAllMocks());

describe("uploaded media attachments", () => {
  it("controls the real audio resource instead of a timer", async () => {
    const play = vi
      .spyOn(HTMLMediaElement.prototype, "play")
      .mockResolvedValue(undefined);
    const user = userEvent.setup();
    const { container } = render(
      <AudioAttachment
        part={{ kind: "audio", name: "voice.wav", url: "/assets/audio" }}
      />,
    );

    const audio = container.querySelector("audio");
    expect(audio).toHaveAttribute("src", "/assets/audio");
    await user.click(screen.getByRole("button", { name: "Odtwórz voice.wav" }));
    expect(play).toHaveBeenCalledOnce();

    fireEvent.play(audio!);
    expect(screen.getByRole("button", { name: "Wstrzymaj voice.wav" })).toBeInTheDocument();
  });

  it("labels a composer recording as a voice message instead of exposing its generated file name", () => {
    render(
      <AudioAttachment
        part={{
          kind: "audio",
          name: "wiadomosc-glosowa-2026.webm",
          url: "/assets/voice",
          duration: "0:04",
          voiceMessage: true,
        }}
      />,
    );

    expect(screen.getByText("Wiadomość głosowa")).toBeInTheDocument();
    expect(screen.queryByText("wiadomosc-glosowa-2026.webm")).not.toBeInTheDocument();
    expect(screen.queryByRole("link", { name: /Pobierz/ })).not.toBeInTheDocument();
  });

  it("clears playback failure when a refreshed attachment receives a new URL", () => {
    const { container, rerender } = render(
      <AudioAttachment
        part={{ kind: "audio", name: "voice.wav", url: "/assets/expired" }}
      />,
    );
    const audio = container.querySelector("audio")!;
    fireEvent.error(audio);
    expect(screen.getByRole("button", { name: "Nie można odtworzyć voice.wav" }))
      .toBeDisabled();

    rerender(
      <AudioAttachment
        part={{ kind: "audio", name: "voice.wav", url: "/assets/refreshed" }}
      />,
    );

    expect(screen.getByRole("button", { name: "Odtwórz voice.wav" })).toBeEnabled();
    expect(screen.queryByRole("alert")).toBeNull();
    expect(container.querySelector("audio")).toHaveAttribute(
      "src",
      "/assets/refreshed",
    );
  });

  it("plays the uploaded video inline with Stellarr controls, its poster and sound", () => {
    render(<Attachment part={{ kind: "video", name: "clip.mp4", url: "/clip.mp4", thumb: "/poster.jpg" }} />);
    const video = screen.getByLabelText("clip.mp4");
    expect(screen.getAllByRole("button", { name: "Odtwórz clip.mp4" })).toHaveLength(2);
    expect(video).toHaveProperty("controls", false);
    expect(video).toHaveProperty("muted", false);
    expect(video).toHaveProperty("autoplay", false);
    expect(video).toHaveAttribute("poster", "/poster.jpg");
    expect(video).toHaveAttribute("src", "/clip.mp4");
  });

  it("seeks against the video's measured duration and reflects playback events", () => {
    render(<VideoAttachment part={{ kind: "video", name: "clip.mp4", url: "/clip.mp4" }} />);
    const video = screen.getByLabelText<HTMLVideoElement>("clip.mp4");
    const timeline = screen.getByRole("slider", { name: /Pozycja odtwarzania/ });
    expect(timeline).toBeDisabled();
    Object.defineProperty(video, "duration", { value: 80 });
    Object.defineProperties(video, { videoWidth: { value: 360 }, videoHeight: { value: 640 } });
    fireEvent.loadedMetadata(video);
    expect(video.closest(".att-video")).toHaveStyle("--video-ratio: 0.5625");
    fireEvent.change(timeline, { target: { value: "60" } });
    expect(video.currentTime).toBe(60);
    expect(screen.getByText("1:00 / 1:20")).toBeInTheDocument();
    fireEvent.play(video);
    expect(screen.getAllByRole("button", { name: "Wstrzymaj clip.mp4" })).toHaveLength(2);
    fireEvent.ended(video);
    expect(screen.getAllByRole("button", { name: "Odtwórz clip.mp4" })).toHaveLength(2);
  });

  it("offers the original when a codec fails and tries a refreshed URL", () => {
    const part = { kind: "video" as const, name: "clip.mp4", url: "/clip.mp4" };
    const { rerender } = render(<VideoAttachment part={part} />);
    fireEvent.error(screen.getByLabelText("clip.mp4"));
    expect(screen.getByRole("link", { name: "Pobierz" })).toHaveAttribute("href", part.url);
    rerender(<VideoAttachment part={{ ...part, url: "/new.mp4" }} />);
    expect(screen.getByLabelText("clip.mp4")).toHaveAttribute("src", "/new.mp4");
  });

  it.each(["audio", "video"] as const)("decrypts private %s only on play and can retry", async (kind) => {
    const resolveOriginal = vi.fn().mockRejectedValueOnce(new Error("offline")).mockResolvedValue("blob:opened");
    const { container } = render(
      <Attachment part={{ kind, name: "clip", url: "e2ee:id" }} callbacks={{ resolveOriginal }} />,
    );
    expect(container.querySelector(kind)).toBeNull();
    expect(resolveOriginal).not.toHaveBeenCalled();
    const user = userEvent.setup();
    const play = screen.getByRole("button", { name: "Odtwórz clip" });
    await user.click(play);
    expect(screen.getByRole("status")).toHaveTextContent("Błąd odtwarzania");
    await user.click(play);
    expect(resolveOriginal).toHaveBeenCalledWith("e2ee:id");
    expect(container.querySelector(kind)).toHaveAttribute("src", "blob:opened");
    expect(container.querySelector(kind)).toHaveProperty("autoplay", true);
  });

  it("changes audio volume and restores the same level after unmuting", async () => {
    const { container } = render(<AudioAttachment part={{ kind: "audio", name: "song", url: "/song" }} />);
    const audio = container.querySelector("audio")!;
    fireEvent.change(screen.getByRole("slider", { name: "Głośność" }), { target: { value: "35" } });
    expect(audio.volume).toBe(0.35);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Wycisz dźwięk" }));
    expect(audio.muted).toBe(true);
    await user.click(screen.getByRole("button", { name: "Włącz dźwięk" }));
    expect(audio.muted).toBe(false);
    expect(audio.volume).toBe(0.35);
  });
});
