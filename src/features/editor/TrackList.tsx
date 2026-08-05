import type { Track } from "../../lib/types";

interface TrackListProps {
  tracks: Track[];
  selected: number;
  onSelect: (index: number) => void;
  onAdd: () => void;
  onDelete: (trackId: string) => void;
}

/** The project's tracks, and the two things you can do to the list itself. */
export function TrackList({ tracks, selected, onSelect, onAdd, onDelete }: TrackListProps) {
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
              className={`tracklist__item ${index === selected ? "tracklist__item--selected" : ""}`}
              role="button"
              tabIndex={0}
              onClick={() => onSelect(index)}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
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
                  onClick={(e) => {
                    e.stopPropagation();
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
