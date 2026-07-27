import type { Track } from "../../lib/types";
import "./TrackList.css";

interface TrackListProps {
  tracks: Track[];
  selectedTrack: number;
  onSelect: (index: number) => void;
  onAdd: () => void;
  onDelete: (trackId: string) => void;
}

/**
 * The track column: swatch, name, note count, and a delete button that only appears on
 * hover or focus so a mis-click cannot cost a track.
 *
 * The delete button is hidden entirely when there is one track left, because a project
 * must keep one — refusing afterwards would be a worse way to say so.
 */
export function TrackList({ tracks, selectedTrack, onSelect, onAdd, onDelete }: TrackListProps) {
  return (
    <>
      <div className="editor__panel-head">
        <h2 className="editor__panel-title">Tracks</h2>
        <button className="btn btn--ghost" onClick={onAdd}>
          + Add
        </button>
      </div>

      <ul className="tracklist">
        {tracks.map((track, index) => (
          <li key={track.id}>
            <div
              className={`tracklist__item ${index === selectedTrack ? "tracklist__item--selected" : ""}`}
              role="button"
              tabIndex={0}
              onClick={() => onSelect(index)}
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") {
                  event.preventDefault();
                  onSelect(index);
                }
              }}
            >
              <span className="tracklist__swatch" style={{ background: track.color }} />
              <span className="tracklist__name truncate">{track.name}</span>
              <span className="tracklist__count mono muted">{track.notes.length}</span>
              {tracks.length > 1 && (
                <button
                  className="btn btn--ghost btn--icon tracklist__remove"
                  onClick={(event) => {
                    event.stopPropagation();
                    onDelete(track.id);
                  }}
                  aria-label={`Delete ${track.name}`}
                >
                  ✕
                </button>
              )}
            </div>
          </li>
        ))}
      </ul>
    </>
  );
}
