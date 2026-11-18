import { useState } from "react";

import type { GalleryImage, GalleryPart, ImagePart } from "@/lib/messages/parts";

import { reservedImageStyle } from "./imageBox";
import { placeholderStyle } from "./placeholderStyle";

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
          <GalleryTile
            key={`${image.src}-${index}`}
            image={image}
            index={index}
            onClick={onOpen && (() => onOpen(part.images, index))}
          />
        ))}
      </div>
    );
  }
  return <LoneImage part={part} onOpen={onOpen} />;
}

function GalleryTile({
  image,
  index,
  onClick,
}: {
  image: GalleryImage;
  index: number;
  onClick?(): void;
}) {
  // The stand-in is the tile's background until the file is in, and goes then:
  // a picture with transparency must not show the blur through itself.
  const shown = image.preview ?? image.src;
  const [settled, setSettled] = useState<string | null>(null);
  const settle = () => setSettled(shown);
  return (
    <img
      style={settled === shown ? undefined : placeholderStyle(image.placeholder)}
      src={shown}
      alt={image.alt}
      data-media-index={index}
      width={TILE_PX}
      height={TILE_PX}
      loading="lazy"
      onLoad={settle}
      onError={settle}
      onClick={onClick}
    />
  );
}

function LoneImage({ part, onOpen }: { part: ImagePart; onOpen: ImageAttachmentProps["onOpen"] }) {
  // The size measured on upload holds the image's box while it loads, so the
  // row does not grow under the reader when it arrives. Once the file is in
  // (or has failed), its own size takes over: a measurement that disagrees
  // with the file can cost a shift, never a letterboxed picture.
  const shown = part.preview ?? part.src;
  const [settled, setSettled] = useState<string | null>(null);
  const reserved = settled === shown ? undefined : reservedImageStyle(part.width, part.height);
  const stand = settled === shown ? undefined : placeholderStyle(part.placeholder);
  const settle = () => setSettled(shown);

  // A lone image is a gallery of one, so the viewer takes the same shape
  // and simply has no neighbours to step to.
  const only: GalleryImage[] = [
    { src: part.src, alt: part.alt, ...(part.preview === undefined ? {} : { preview: part.preview }) },
  ];

  return (
    <div className="att">
      <img
        className={reserved === undefined ? "att-image" : "att-image reserved"}
        style={reserved === undefined ? stand : { ...reserved, ...stand }}
        src={shown}
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
