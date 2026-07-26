import { useState } from "react";

import type { Pending } from "./pending";
import "./ReviewBar.css";

interface ReviewBarProps {
  pending: Pending;
  onApply: () => void;
  onDiscard: () => void;
  /** Open the waveform editor. Only offered for a transcription. */
  onFineTune: () => void;
}

/**
 * One review surface for both ways of producing notes.
 *
 * An AI edit and a transcription are the same shape of interaction — something produced
 * notes you did not type, you look at them on the roll, you accept or reject — so they
 * get the same bar rather than two panels that happen to rhyme. It replaces the prompt
 * bar while a decision is outstanding, which is also how the app enforces one decision
 * at a time.
 */
export function ReviewBar({ pending, onApply, onDiscard, onFineTune }: ReviewBarProps) {
  const [showSteps, setShowSteps] = useState(false);
  if (!pending) return null;

  if (pending.kind === "ai") {
    const { proposal } = pending;
    const { diff } = proposal;

    return (
      <div className="review">
        <div className="review__body">
          {proposal.narration && <p className="review__narration">{proposal.narration}</p>}

          {proposal.empty ? (
            <span className="review__hint">Nothing to apply.</span>
          ) : (
            <div className="review__counts">
              {diff.added.length > 0 && (
                <span className="review__count review__count--added">+{diff.added.length}</span>
              )}
              {diff.removed.length > 0 && (
                <span className="review__count review__count--removed">−{diff.removed.length}</span>
              )}
              {diff.changed.length > 0 && (
                <span className="review__count review__count--changed">
                  ~{diff.changed.length}
                </span>
              )}
              <span className="review__hint">
                green is new, red is going, amber is moving
              </span>
            </div>
          )}

          {showSteps && proposal.steps.length > 0 && (
            <ol className="review__steps">
              {proposal.steps.map((step, index) => (
                <li key={index} className={step.ok ? "" : "review__step--failed"}>
                  <span className="mono">{step.tool}</span> — {step.result}
                </li>
              ))}
            </ol>
          )}
        </div>

        <div className="review__actions">
          {proposal.steps.length > 0 && (
            <button
              className="btn btn--ghost"
              onClick={() => setShowSteps((v) => !v)}
              aria-expanded={showSteps}
            >
              {showSteps ? "Hide" : "Show"} {proposal.steps.length} step
              {proposal.steps.length === 1 ? "" : "s"}
            </button>
          )}
          <span className="review__usage mono">
            {proposal.usage.input_tokens.toLocaleString()} in ·{" "}
            {proposal.usage.output_tokens.toLocaleString()} out
          </span>
          {!proposal.empty && (
            <button className="btn btn--primary" onClick={onApply}>
              Apply
            </button>
          )}
          <button className="btn" onClick={onDiscard}>
            {proposal.empty ? "Dismiss" : "Discard"}
          </button>
        </div>
      </div>
    );
  }

  const { preview } = pending;
  const outOfTune = preview.notes.filter((n) => Math.abs(n.cents_off) > 35).length;

  return (
    <div className="review">
      <div className="review__body">
        {preview.warning ? (
          <p className="review__warning">{preview.warning}</p>
        ) : (
          <p className="review__narration">
            {preview.notes.length} note{preview.notes.length === 1 ? "" : "s"} from{" "}
            {preview.duration_seconds.toFixed(1)}s
            {preview.tempo_estimated
              ? ` at an estimated ${preview.tempo_bpm.toFixed(0)} bpm`
              : ""}
            .
          </p>
        )}

        <div className="review__counts">
          <span className="review__count review__count--added">+{preview.notes.length}</span>
          <span className="review__hint">
            {Math.round(preview.pitched_fraction * 100)}% pitched
            {outOfTune > 0 && ` · ${outOfTune} more than a third of a semitone off`}
          </span>
        </div>
      </div>

      <div className="review__actions">
        <button className="btn" onClick={onFineTune}>
          Fine-tune…
        </button>
        {preview.notes.length > 0 && (
          <button className="btn btn--primary" onClick={onApply}>
            Add to track
          </button>
        )}
        <button className="btn" onClick={onDiscard}>
          Discard
        </button>
      </div>
    </div>
  );
}
