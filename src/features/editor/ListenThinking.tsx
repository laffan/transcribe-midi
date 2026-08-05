import { useEffect, useState } from "react";

interface ListenThinkingProps {
  prompt: string;
  onCancel: () => void;
}

/**
 * The wait while a described edit is being worked out.
 *
 * It covers the editor, and that is the point rather than a side effect. A proposal is a
 * transaction against the notes as they were when it was asked for: edit underneath it
 * and the answer arrives stale, which Rust correctly refuses — so the request is thrown
 * away and the wait was for nothing. Taking the surface away for the duration turns "you
 * may not edit now" from a rule nobody was told into something obvious.
 *
 * The bar is indeterminate on purpose. There is nothing honest to measure: a tool loop
 * takes as many turns as it takes, and a bar that guessed would be a bar that lied.
 */
export function ListenThinking({ prompt, onCancel }: ListenThinkingProps) {
  const [seconds, setSeconds] = useState(0);

  useEffect(() => {
    const timer = window.setInterval(() => setSeconds((at) => at + 0.1), 100);
    return () => window.clearInterval(timer);
  }, []);

  return (
    <>
      <div className="listen__body listen__body--working">
        <div className="working">
          <p className="working__title">Working out what you asked for</p>
          <p className="thinking__prompt">“{prompt}”</p>

          <div className="working__bar" role="progressbar" aria-label="Thinking">
            <span className="working__fill working__fill--indeterminate" />
          </div>

          <p className="working__detail mono">{seconds.toFixed(1)}s</p>
          <p className="field__hint">
            It may call the editing tools several times before it answers. Nothing is
            applied until you accept it, and you will hear it first.
          </p>
        </div>
      </div>

      <footer className="listen__actions">
        <span className="field__hint">
          The editor is covered while this runs: a suggestion is written against the notes
          as they were when you asked, so changing them underneath would waste the answer.
        </span>
        <div className="spacer" />
        <button className="btn" onClick={onCancel}>
          Stop waiting
        </button>
      </footer>
    </>
  );
}
