import { memo } from "react";

import type { CodeToken } from "@/lib/messages/codeLanguages";

export type CodeRun = Pick<CodeToken, "kind" | "text">;

/** Cuts the highlighted runs at line breaks, each piece keeping its own class. */
export function splitCodeLines(tokens: readonly CodeToken[]): CodeRun[][] {
  const lines: CodeRun[][] = [[]];
  for (const token of tokens) {
    const pieces = token.text.split("\n");
    pieces.forEach((text, index) => {
      if (index > 0) lines.push([]);
      if (text.length > 0) lines.at(-1)?.push({ kind: token.kind, text });
    });
  }
  return lines;
}

function sameRuns(a: readonly CodeRun[], b: readonly CodeRun[]): boolean {
  return (
    a.length === b.length &&
    a.every((run, index) => run.kind === b[index]?.kind && run.text === b[index]?.text)
  );
}

interface CodeLineProps {
  runs: readonly CodeRun[];
  className?: string;
}

/**
 * One line of highlighted code.
 *
 * Compared by its content rather than by identity: the field in the composer
 * tokenizes the whole draft on every keystroke, and only the line being typed
 * on should render again.
 */
export const CodeLine = memo(
  function CodeLine({ runs, className }: CodeLineProps) {
    return (
      <span className={className}>
        {runs.map((run, index) =>
          run.kind === "text" ? (
            run.text
          ) : (
            <span key={index} className={run.kind}>
              {run.text}
            </span>
          ),
        )}
      </span>
    );
  },
  (previous, next) =>
    previous.className === next.className && sameRuns(previous.runs, next.runs),
);
