import { useLanguage } from "@/state/LanguageProvider";

export interface TypingProps {
  /** Who is typing right now. Empty keeps the row mounted but hidden. */
  names: string[];
}

/** "Ada", "Ada and Bo", "Ada, Bo and Cy" with the verb agreeing in number. */
function useSentence(names: string[]): string {
  const { t } = useLanguage();
  const [first, ...others] = names;
  if (first === undefined) return "";
  if (others.length === 0) return t("typing.one", { name: first });

  const last = others.pop();
  return t("typing.many", { names: `${[first, ...others].join(", ")} ${t("common.and")} ${last}` });
}

/**
 * The live region stays mounted so assistive technology announces new names.
 * Its row takes up space only while someone is typing.
 */
export function Typing({ names }: TypingProps) {
  const sentence = useSentence(names);
  return (
    <div className={names.length > 0 ? "typing show" : "typing"} aria-live="polite">
      <span className="dots" aria-hidden="true">
        <i />
        <i />
        <i />
      </span>
      <span>{sentence}</span>
    </div>
  );
}
