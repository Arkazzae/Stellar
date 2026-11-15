"use client";

import { useState } from "react";

import type { GalleryImage, GalleryPart, ImagePart } from "../parts";

import { reservedImageStyle } from "./imageBox";

/** Gallery tiles are square thumbnails of this size. */
const TILE_PX = 200;

export interface ImageAttachmentProps {
  part: ImagePart | GalleryPart;
  /** Opens the lightbox; without it the images are inert. */
  onOpen?(images: GalleryImage[], index: number): void;
}

export function ImageAttachment({ part, onOpen }: ImageAttachmentProps) {
  if (part.kind === "gallery") {
    return (
      <div className="att att-gallery">
        {part.images.map((image, index) => (
          <img
            key={`${image.src}-${index}`}
            src={image.src}
            alt={image.alt}
            data-media-index={index}
            width={TILE_PX}
            height={TILE_PX}
            loading="lazy"
            onClick={onOpen && (() => onOpen(part.images, index))}
          />
        ))}
      </div>
    );
  }
  return <LoneImage part={part} onOpen={onOpen} />;
}

function LoneImage({ part, onOpen }: { part: ImagePart; onOpen: ImageAttachmentProps["onOpen"] }) {
  // The size measured on upload holds the image's box while it loads, so the
  // row does not grow under the reader when it arrives. Once the file is in
  // (or has failed), its own size takes over: a measurement that disagrees
  // with the file can cost a shift, never a letterboxed picture.
  const [settled, setSettled] = useState<string | null>(null);
  const reserved =
    settled === part.src ? undefined : reservedImageStyle(part.width, part.height);
  const settle = () => setSettled(part.src);

  // A lone image is a gallery of one, so the viewer takes the same shape
  // and simply has no neighbours to step to.
  const only: GalleryImage[] = [{ src: part.src, alt: part.alt }];

  return (
    <div className="att">
      <img
        className={reserved === undefined ? "att-image" : "att-image reserved"}
        style={reserved}
        src={part.src}
        alt={part.alt}
        data-media-index={0}
        width={part.width}
        height={part.height}
        loading="lazy"
        onLoad={settle}
        onError={settle}
        onClick={onOpen && (() => onOpen(only, 0))}
      />
    </div>
  );
}
