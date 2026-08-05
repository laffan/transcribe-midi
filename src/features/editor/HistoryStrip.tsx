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
 * Consecutive identical steps, folded into one row and a count.
 *
 * Drawing five notes by hand produces five rows reading "Insert note", which is a wall
 * of the same word where the point of the panel is the *shape* of what you did —
 * "quantised 1/16 → harmonised a third → humanised". A run says one thing, so it gets
 * one row; the count is what tells you how many are in it.
 *
 * Only consecutive runs fold. Two separate bursts of drawing with an edit between them
 * are two different moments and stay two rows.
 */
interface Run {
  label: string;
  count: number;
  /** Index of the run's most recent step, for keying and for the Undo affordance. */
  end: number;
}

export function foldRuns(labels: string[]): Run[] {
  const runs: Run[] = [];
  labels.forEach((label, index) => {
    const last = runs[runs.length - 1];
    if (last && last.label === label) {
      last.count += 1;
      last.end = index;
    } else {
      runs.push({ label, count: 1, end: index });
    }
  });
  return runs;
}

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

      {foldRuns(shown).map((run, index, runs) => {
        // Undo still takes back one step, so a run of five shrinks to four rather than
        // vanishing. The count is what makes that legible instead of surprising.
        const isLatest = index === runs.length - 1;
        return (
          <li key={`${run.end}-${run.label}`} className="history__item">
            <span className="history__label truncate" title={run.label}>
              {run.label}
            </span>
            {run.count > 1 && <span className="history__count mono">×{run.count}</span>}
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
      {foldRuns(redoHistory.slice().reverse()).map((run, index) => (
        <li key={`redo-${run.end}-${run.label}`} className="history__item history__item--undone">
          <span className="history__label truncate" title={run.label}>
            {run.label}
          </span>
          {run.count > 1 && <span className="history__count mono">×{run.count}</span>}
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
