import type { ReactNode } from "react";

import "./ListenOverlay.css";

interface ListenOverlayProps {
  /**
   * Which activity this is. Two use the same frame — turning audio into notes, and
   * describing an edit — and the marker is how you know at a glance which one you are in.
   */
  kind?: "Listen" | "Describe";
  /** Short line naming what is happening, shown next to the marker. */
  title: string;
  /** Numbers about the take: length, notes, tempo. */
  stat?: ReactNode;
  /** Omitted while the take is being read: there is nothing to go back to yet. */
  onClose?: () => void;
  closeLabel?: string;
  /** A body and a footer, supplied by whichever stage is running. */
  children: ReactNode;
}

/**
 * The frame around turning audio into notes, from the first moment.
 *
 * Listening used to be a strip along the bottom that only became a full view once the
 * transcription existed — which put the least reversible part of the whole flow, the
 * performance itself, in the smallest space on screen. It is one continuous activity:
 * you play something, you look at what came back, you fix it or you do it again. So it
 * is one surface for the duration, and the stage inside it changes.
 */
export function ListenOverlay({
  kind = "Listen",
  title,
  stat,
  onClose,
  closeLabel,
  children,
}: ListenOverlayProps) {
  return (
    <div className="listen" role="dialog" aria-label={kind}>
      <header className="listen__head">
        <span className="listen__stage">{kind}</span>
        <h2 className="listen__title truncate">{title}</h2>
        {stat && <span className="listen__stat mono">{stat}</span>}
        <div className="spacer" />
        {onClose && (
          <button
            className="btn btn--ghost btn--icon"
            onClick={onClose}
            aria-label={closeLabel}
            title={closeLabel}
          >
            ✕
          </button>
        )}
      </header>

      {children}
    </div>
  );
}
