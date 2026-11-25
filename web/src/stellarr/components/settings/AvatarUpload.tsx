import { useEffect, useId, useRef, useState, type ReactNode } from "react";

import { Button, cx } from "@stellarr/ui";
import { Icon } from "@stellarr/icons";
import { ConfirmError } from "@/components/primitives/ConfirmDialog";
import card from "@/components/primitives/ConfirmDialog.module.css";
import { Modal } from "@/components/primitives/Modal";
import { useAsyncAction } from "@/hooks/useAsyncAction";
import { ANIMATION_HEAD_BYTES, isAnimatedImage } from "@/lib/media/imagePreview";
import { useLanguage } from "@/state/hooks";

import styles from "./AvatarUpload.module.css";

const TYPES = ["image/jpeg", "image/png", "image/gif", "image/webp"];
const MAX_BYTES = 4 * 1024 * 1024;
const OUTPUT_SIZE = 512;

interface Point {
  x: number;
  y: number;
}

const CENTRE: Point = { x: 0.5, y: 0.5 };
const KEY_MOVES: Record<string, Point> = {
  ArrowLeft: { x: -0.05, y: 0 },
  ArrowRight: { x: 0.05, y: 0 },
  ArrowUp: { x: 0, y: -0.05 },
  ArrowDown: { x: 0, y: 0.05 },
};

interface Dimensions {
  width: number;
  height: number;
}

interface Crop extends Point {
  side: number;
}

function cropOf({ width, height }: Dimensions, centre: Point, zoom: number): Crop {
  const side = Math.min(width, height) / zoom;
  return {
    x: Math.max(0, Math.min(width - side, centre.x * width - side / 2)),
    y: Math.max(0, Math.min(height - side, centre.y * height - side / 2)),
    side,
  };
}

async function exportCrop(image: HTMLImageElement, { x, y, side }: Crop): Promise<File> {
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = Math.min(OUTPUT_SIZE, Math.max(1, Math.round(side)));
  const context = canvas.getContext("2d");
  if (context === null) throw new Error("Canvas unavailable");
  context.imageSmoothingQuality = "high";
  context.drawImage(image, x, y, side, side, 0, 0, canvas.width, canvas.height);
  const output = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, "image/webp", 0.9));
  if (output === null || output.size > MAX_BYTES) throw new Error("Could not encode avatar");
  return new File([output], output.type === "image/webp" ? "avatar.webp" : "avatar.png", { type: output.type });
}

interface AvatarUploadProps {
  children: ReactNode;
  label: string;
  shape: "circle" | "square";
  className?: string;
  disabled?: boolean;
  onSave: (file: File) => Promise<void> | void;
}

export function AvatarUpload({ children, label, shape, className, disabled, onSave }: AvatarUploadProps) {
  const { t } = useLanguage();
  const input = useRef<HTMLInputElement>(null);
  const [file, setFile] = useState<File | null>(null);
  const [error, setError] = useState<string | null>(null);

  const pick = (next: File) => {
    setError(null);
    if (!TYPES.includes(next.type)) setError(t("avatar.format"));
    else if (next.size > MAX_BYTES) setError(t("avatar.size"));
    else setFile(next);
  };

  return (
    <>
      <button
        type="button"
        aria-label={label}
        title={label}
        disabled={disabled}
        className={cx(styles.trigger, className)}
        data-crop-shape={shape}
        onClick={(event) => {
          event.currentTarget.focus();
          input.current?.click();
        }}
      >
        {children}
        <span className={styles.veil} aria-hidden="true"><Icon name="camera" size={18} /></span>
      </button>
      <input
        ref={input}
        type="file"
        accept={TYPES.join(",")}
        disabled={disabled}
        hidden
        onChange={(event) => {
          const next = event.currentTarget.files?.[0];
          event.currentTarget.value = "";
          if (next !== undefined) pick(next);
        }}
      />
      {error === null ? null : <p className={styles.error} role="alert">{error}</p>}
      {file === null ? null : (
        <CropDialog file={file} shape={shape} onSave={onSave} onClose={() => setFile(null)} />
      )}
    </>
  );
}

function CropDialog({ file, shape, onSave, onClose }: {
  file: File;
  shape: AvatarUploadProps["shape"];
  onSave: AvatarUploadProps["onSave"];
  onClose: () => void;
}) {
  const { t } = useLanguage();
  const titleId = useId();
  const detailId = useId();
  const action = useAsyncAction();
  const image = useRef<HTMLImageElement>(null);
  const drag = useRef<{ pointer: number; at: Point } | null>(null);
  const [dimensions, setDimensions] = useState<Dimensions | null>(null);
  const [decodeFailed, setDecodeFailed] = useState(false);
  const [animated, setAnimated] = useState(false);
  const [zoom, setZoom] = useState(1);
  const [centre, setCentre] = useState(CENTRE);
  const crop = dimensions === null ? null : cropOf(dimensions, centre, zoom);

  useEffect(() => {
    const preview = image.current;
    if (preview === null) return;
    const objectUrl = URL.createObjectURL(file);
    preview.src = objectUrl;
    let active = true;
    void Promise.resolve()
      .then(() => file.slice(0, ANIMATION_HEAD_BYTES).arrayBuffer())
      .then((buffer) => { if (active) setAnimated(isAnimatedImage(new Uint8Array(buffer))); })
      .catch(() => undefined);
    return () => {
      active = false;
      preview.removeAttribute("src");
      URL.revokeObjectURL(objectUrl);
    };
  }, [file]);

  const move = (x: number, y: number) => {
    if (dimensions === null || action.pending) return;
    setCentre((previous) => {
      const current = cropOf(dimensions, previous, zoom);
      const next = cropOf(dimensions, {
        x: (current.x + (0.5 - x) * current.side) / dimensions.width,
        y: (current.y + (0.5 - y) * current.side) / dimensions.height,
      }, zoom);
      return { x: (next.x + next.side / 2) / dimensions.width, y: (next.y + next.side / 2) / dimensions.height };
    });
  };

  const save = async (original: boolean) => {
    if (crop === null || action.pending || image.current === null) return;
    const preview = image.current;
    const saved = await action.run(async () => onSave(original ? file : await exportCrop(preview, crop)));
    if (saved) onClose();
  };

  return (
    <Modal open onClose={() => { if (!action.pending) onClose(); }} labelledBy={titleId} className={card.wrap ?? ""}>
      <div className={card.card} aria-busy={action.pending || undefined}>
        <div className={card.body}>
          <h2 id={titleId}>{t("avatar.crop")}</h2>
          <p id={detailId} className={card.sub}>{t("avatar.cropDetail")}</p>
          <div
            className={styles.crop}
            data-crop-shape={shape}
            role="group"
            tabIndex={0}
            aria-label={t("avatar.position")}
            aria-describedby={detailId}
            onKeyDown={(event) => {
              const offset = KEY_MOVES[event.key];
              if (offset === undefined) return;
              event.preventDefault();
              move(offset.x, offset.y);
            }}
            onPointerDown={(event) => {
              if (action.pending || crop === null || drag.current !== null || event.button !== 0) return;
              event.currentTarget.setPointerCapture(event.pointerId);
              drag.current = { pointer: event.pointerId, at: { x: event.clientX, y: event.clientY } };
            }}
            onPointerMove={(event) => {
              const from = drag.current;
              if (from?.pointer !== event.pointerId) return;
              const width = event.currentTarget.getBoundingClientRect().width;
              move((event.clientX - from.at.x) / width, (event.clientY - from.at.y) / width);
              drag.current = { pointer: event.pointerId, at: { x: event.clientX, y: event.clientY } };
            }}
            onLostPointerCapture={() => { drag.current = null; }}
          >
            <img
              ref={image}
              alt=""
              draggable={false}
              style={dimensions === null || crop === null ? { visibility: "hidden" } : {
                width: (dimensions.width / crop.side * 100) + "%",
                left: (-crop.x / crop.side * 100) + "%",
                top: (-crop.y / crop.side * 100) + "%",
              }}
              onLoad={(event) => {
                const { naturalWidth: width, naturalHeight: height } = event.currentTarget;
                if (width === 0 || height === 0) setDecodeFailed(true);
                else setDimensions({ width, height });
              }}
              onError={() => setDecodeFailed(true)}
            />
          </div>
          <div className={styles.adjust}>
            <label className={styles.zoom}>
              {t("avatar.zoom")}
              <input
                type="range"
                min={1}
                max={3}
                step={0.01}
                value={zoom}
                aria-valuetext={Math.round(zoom * 100) + "%"}
                disabled={crop === null || action.pending}
                onChange={(event) => setZoom(event.currentTarget.valueAsNumber)}
              />
            </label>
            <Button type="button" variant="quiet" size="sm" disabled={crop === null || action.pending} onClick={() => setCentre(CENTRE)}>
              {t("avatar.centre")}
            </Button>
          </div>
          {animated ? (
            <div className={styles.original}>
              <p className={card.hint}>{t("avatar.animated")}</p>
              <Button type="button" variant="secondary" size="sm" disabled={crop === null || action.pending} onClick={() => void save(true)}>
                {t("avatar.original")}
              </Button>
            </div>
          ) : null}
          {decodeFailed ? <ConfirmError text={t("avatar.decodeFailed")} /> : null}
          {action.failed ? <ConfirmError text={t("avatar.saveFailed")} /> : null}
        </div>
        <div className={card.foot}>
          <Button type="button" variant="secondary" disabled={action.pending} onClick={onClose}>{t("common.cancel")}</Button>
          <Button type="button" variant="primary" disabled={crop === null || decodeFailed || action.pending} onClick={() => void save(false)}>
            {action.pending ? t("common.saving") : t("common.save")}
          </Button>
        </div>
      </div>
    </Modal>
  );
}
