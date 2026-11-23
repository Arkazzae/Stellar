import { useRef, useState } from "react";

import { Button } from "@stellarr/ui";
import { Icon } from "@stellarr/icons";
import { useConfirm } from "@/components/primitives/ConfirmDialog";
import {
  THEME_FILE_MAX_BYTES,
  readThemeFile,
  themeFileName,
  themeFileOf,
  themeFileText,
  type ThemeFileProblem,
} from "@/design/themeFile";
import type { ThemePreference } from "@/design/themes";
import type { MessageKey } from "@/i18n";
import { useLanguage } from "@/state/hooks";

import styles from "./AppearancePane.module.css";
import { customThemeName } from "./PaletteBoard";

const PROBLEMS: Record<ThemeFileProblem | "unreadable", MessageKey> = {
  notOurs: "theme.file.notOurs",
  colours: "theme.file.colours",
  tooLarge: "theme.file.tooLarge",
  unreadable: "theme.file.unreadable",
};

/** Hands the browser a file to save, under the given name. */
function download(name: string, text: string) {
  const url = URL.createObjectURL(new Blob([text], { type: "application/json" }));
  const link = document.createElement("a");
  link.href = url;
  link.download = name;
  link.click();
  // The click has started the download; the address is not needed after it.
  setTimeout(() => URL.revokeObjectURL(url), 0);
}

export interface ThemeFileRowProps {
  theme: ThemePreference;
  onChange: (patch: Partial<ThemePreference>) => void;
}

/**
 * The theme as a file: the palette and accent in use saved to disk, and a
 * file picked from disk applied as the person's own palette. Applying one
 * replaces their own palette, so it asks first when there is one.
 */
export function ThemeFileRow({ theme, onChange }: ThemeFileRowProps) {
  const { t } = useLanguage();
  const confirm = useConfirm();
  const input = useRef<HTMLInputElement>(null);
  const [problem, setProblem] = useState<MessageKey | null>(null);

  const save = () => {
    const file = themeFileOf(theme);
    download(themeFileName(file.name, t("theme.custom.untitled")), themeFileText(file));
  };

  const load = async (file: File) => {
    setProblem(null);
    if (file.size > THEME_FILE_MAX_BYTES) {
      setProblem(PROBLEMS.tooLarge);
      return;
    }
    let text: string;
    try {
      text = await file.text();
    } catch {
      setProblem(PROBLEMS.unreadable);
      return;
    }
    const read = readThemeFile(text);
    if (!read.ok) {
      setProblem(PROBLEMS[read.problem]);
      return;
    }
    if (
      theme.custom !== null &&
      !(await confirm({
        title: t("theme.custom.replaceTitle"),
        description: t("theme.file.replaceDetail"),
        subject: { kind: "item", name: customThemeName(theme.custom, t("theme.custom.untitled")), mark: { icon: "palette" } },
        confirmLabel: t("theme.custom.replaceConfirm"),
      }))
    ) {
      return;
    }
    onChange(read.patch);
  };

  return (
    <div className="frm-row ap-row">
      <span className="frm-lab">
        {t("theme.file.title")}
        <span className={problem === null ? undefined : "frm-detail-error"} role={problem === null ? undefined : "alert"}>
          {problem === null ? t("theme.file.detail") : t(problem)}
        </span>
      </span>
      <div className={`frm-ctl ap-end ${styles.actions}`}>
        <Button type="button" variant="secondary" size="sm" onClick={save}>
          <Icon name="download" size={15} />
          {t("theme.file.save")}
        </Button>
        <Button type="button" variant="secondary" size="sm" onClick={() => input.current?.click()}>
          <Icon name="upload" size={15} />
          {t("theme.file.load")}
        </Button>
        <input
          ref={input}
          type="file"
          className="sr-only"
          tabIndex={-1}
          aria-hidden="true"
          accept=".json,application/json"
          onChange={(event) => {
            const file = event.currentTarget.files?.[0];
            // Cleared, so the same file can be picked again after a change.
            event.currentTarget.value = "";
            if (file !== undefined) void load(file);
          }}
        />
      </div>
    </div>
  );
}
