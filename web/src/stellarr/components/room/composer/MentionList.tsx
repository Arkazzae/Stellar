import type { MentionCandidate } from "@/lib/messages/composerMentions";

export function mentionOptionId(listId: string, candidate: MentionCandidate): string {
  return `${listId}-${candidate.id}`;
}

export interface MentionListProps {
  id: string;
  label: string;
  candidates: readonly MentionCandidate[];
  selectedIndex: number;
  onChoose(candidate: MentionCandidate): void;
}

/**
 * The members offered after typing `@`, after `@everyone` and `@here` for
 * whoever may call on the channel. Focus never leaves the textarea: the list
 * is a combobox popup, navigated from the field with the arrow keys, so
 * pressing an option must not blur it.
 */
export function MentionList({
  id,
  label,
  candidates,
  selectedIndex,
  onChoose,
}: MentionListProps) {
  return (
    <div id={id} className="mention-suggestions" role="listbox" aria-label={label}>
      {candidates.map((candidate, index) => (
        <button
          key={candidate.id}
          id={mentionOptionId(id, candidate)}
          type="button"
          role="option"
          aria-selected={index === selectedIndex}
          className={index === selectedIndex ? "selected" : undefined}
          onMouseDown={(event) => event.preventDefault()}
          onClick={() => onChoose(candidate)}
        >
          {candidate.group === undefined ? (
            <>
              <span className="mention-avatar" aria-hidden="true">
                {candidate.displayName.trim().slice(0, 2).toLocaleUpperCase()}
              </span>
              <span className="mention-copy">
                <b>{candidate.displayName}</b>
                <small>@{candidate.username}</small>
              </span>
            </>
          ) : (
            // A group has no face: the word is what is written, and the line
            // under it says what writing it does.
            <>
              <span className="mention-avatar" aria-hidden="true">@</span>
              <span className="mention-copy">
                <b>@{candidate.username}</b>
                <small>{candidate.displayName}</small>
              </span>
            </>
          )}
        </button>
      ))}
    </div>
  );
}
