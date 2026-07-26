import { clearLog, useLog } from "../../lib/console";

/**
 * The optional error console beneath the transport. Newest entry first — during
 * playback and recording the interesting message is always the most recent one, and
 * auto-scrolling a log that is being appended to fights the user.
 */
export function ConsolePanel({ onClose }: { onClose: () => void }) {
  const entries = useLog();

  return (
    <section className="console" aria-label="Console">
      <header className="console__head">
        <h2 className="console__title">Console</h2>
        <span className="console__count muted mono">{entries.length}</span>
        <div className="spacer" />
        <button className="btn btn--ghost" onClick={clearLog} disabled={entries.length === 0}>
          Clear
        </button>
        <button className="btn btn--ghost btn--icon" onClick={onClose} aria-label="Hide console">
          ✕
        </button>
      </header>

      <div className="console__body">
        {entries.length === 0 ? (
          <p className="console__empty muted">Nothing logged yet.</p>
        ) : (
          <ul className="console__list">
            {entries.map((entry) => (
              <li key={entry.id} className={`console__entry console__entry--${entry.level}`}>
                <span className="console__time mono">
                  {new Date(entry.at).toLocaleTimeString(undefined, { hour12: false })}
                </span>
                <span className="console__level">{entry.level}</span>
                <span className="console__message">
                  {entry.message}
                  {entry.detail && <span className="console__detail"> — {entry.detail}</span>}
                </span>
              </li>
            ))}
          </ul>
        )}
      </div>
    </section>
  );
}
