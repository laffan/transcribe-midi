import "./HistoryStrip.css";

interface HistoryStripProps {
  history: string[];
  redoHistory: string[];
  onUndo: () => void;
  onRedo: () => void;
}

/** How many entries to show. Older ones scroll. */
const VISIBLE = 40;

/**
 * The chain of transformations, newest last.
 *
 * When the main way to change notes is describing what you want, the sequence you
 * described is as much the document as the notes are — "quantised 1/16 → harmonised a
 * third → humanised" is a recipe, and a recipe you cannot see is one you cannot reason
 * about. Undo labels already carried this; they were just only visible one at a time.
 */
export function HistoryStrip({ history, redoHistory, onUndo, onRedo }: HistoryStripProps) {
  if (history.length === 0 && redoHistory.length === 0) {
    return (
      <p className="field__hint">
        Nothing yet. Describe an edit below, or press Listen to turn audio into notes.
      </p>
    );
  }

  const shown = history.slice(-VISIBLE);
  const hidden = history.length - shown.length;

  return (
    <ol className="history">
      {hidden > 0 && <li className="history__elision">…and {hidden} earlier</li>}

      {shown.map((label, index) => {
        const isLatest = index === shown.length - 1;
        return (
          <li key={`${index}-${label}`} className="history__item">
            <span className="history__label truncate" title={label}>
              {label}
            </span>
            {isLatest && (
              <button className="btn btn--ghost history__action" onClick={onUndo}>
                Undo
              </button>
            )}
          </li>
        );
      })}

      {/* Undone steps stay visible, dimmed. Losing sight of what you just took back is
          disorienting when the steps are things you asked for in words. */}
      {redoHistory
        .slice()
        .reverse()
        .map((label, index) => (
          <li key={`redo-${index}-${label}`} className="history__item history__item--undone">
            <span className="history__label truncate" title={label}>
              {label}
            </span>
            {index === 0 && (
              <button className="btn btn--ghost history__action" onClick={onRedo}>
                Redo
              </button>
            )}
          </li>
        ))}
    </ol>
  );
}
