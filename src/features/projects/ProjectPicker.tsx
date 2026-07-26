import { useCallback, useEffect, useState } from "react";

import { Modal } from "../../components/Modal";
import { api, errorMessage } from "../../lib/api";
import { logger } from "../../lib/console";
import { DEFAULT_TEMPO, MAX_TEMPO, MIN_TEMPO, type ProjectSummary, type TimeSignature } from "../../lib/types";
import "./ProjectPicker.css";

interface ProjectPickerProps {
  onOpen: (id: string) => void;
  onOpenSettings: () => void;
}

const TIME_SIGNATURES: TimeSignature[] = [
  { numerator: 4, denominator: 4 },
  { numerator: 3, denominator: 4 },
  { numerator: 6, denominator: 8 },
  { numerator: 5, denominator: 4 },
  { numerator: 7, denominator: 8 },
  { numerator: 2, denominator: 4 },
];

function formatWhen(ms: number): string {
  const elapsed = Date.now() - ms;
  const minute = 60_000;
  const hour = 60 * minute;
  const day = 24 * hour;

  if (elapsed < minute) return "just now";
  if (elapsed < hour) return `${Math.floor(elapsed / minute)}m ago`;
  if (elapsed < day) return `${Math.floor(elapsed / hour)}h ago`;
  if (elapsed < 7 * day) return `${Math.floor(elapsed / day)}d ago`;
  return new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
}

export function ProjectPicker({ onOpen, onOpenSettings }: ProjectPickerProps) {
  const [projects, setProjects] = useState<ProjectSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [showNew, setShowNew] = useState(false);
  const [renaming, setRenaming] = useState<ProjectSummary | null>(null);
  const [deleting, setDeleting] = useState<ProjectSummary | null>(null);

  const refresh = useCallback(async () => {
    try {
      const listing = await api.listProjects();
      setProjects(listing.projects);
      // A project directory that failed to parse is dropped from the list rather than
      // failing the whole load — surface it so it does not just silently disappear.
      listing.errors.forEach((e) => logger.error(`Could not read project "${e.id}"`, e.message));
    } catch (error) {
      logger.error("Could not list projects", errorMessage(error));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return (
    <div className="picker">
      <header className="picker__bar">
        <button
          className="btn btn--ghost btn--icon"
          onClick={onOpenSettings}
          aria-label="Settings"
          title="Settings"
        >
          ⚙
        </button>
        <div className="spacer" />
      </header>

      <div className="picker__content">
        <div className="picker__head">
          <div>
            <h1 className="picker__title">Unplugged</h1>
            <p className="picker__subtitle">MIDI sequencing, transcription and notation.</p>
          </div>
          <button className="btn btn--primary btn--lg" onClick={() => setShowNew(true)}>
            New Project
          </button>
        </div>

        <section className="picker__section">
          <h2 className="picker__section-title">Recent</h2>

          {loading ? (
            <p className="picker__empty muted">Loading…</p>
          ) : projects.length === 0 ? (
            <div className="picker__empty">
              <p className="muted">No projects yet.</p>
              <button className="btn" onClick={() => setShowNew(true)}>
                Create your first project
              </button>
            </div>
          ) : (
            <ul className="picker__list">
              {projects.map((project) => (
                <li key={project.id}>
                  <div
                    className="picker__item"
                    role="button"
                    tabIndex={0}
                    onClick={() => onOpen(project.id)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" || e.key === " ") {
                        e.preventDefault();
                        onOpen(project.id);
                      }
                    }}
                  >
                    <div className="picker__item-main">
                      <span className="picker__item-name truncate">{project.name}</span>
                      <span className="picker__item-meta mono">
                        {project.tempo_bpm} BPM · {project.time_signature.numerator}/
                        {project.time_signature.denominator} · {project.track_count}{" "}
                        {project.track_count === 1 ? "track" : "tracks"}
                      </span>
                    </div>

                    <span className="picker__item-when muted">{formatWhen(project.modified_at_ms)}</span>

                    <div className="picker__item-actions" onClick={(e) => e.stopPropagation()}>
                      <button
                        className="btn btn--ghost"
                        onClick={() => setRenaming(project)}
                        aria-label={`Rename ${project.name}`}
                      >
                        Rename
                      </button>
                      <button
                        className="btn btn--ghost picker__delete"
                        onClick={() => setDeleting(project)}
                        aria-label={`Delete ${project.name}`}
                      >
                        Delete
                      </button>
                    </div>
                  </div>
                </li>
              ))}
            </ul>
          )}
        </section>
      </div>

      {showNew && (
        <NewProjectModal
          onClose={() => setShowNew(false)}
          onCreated={(id) => {
            setShowNew(false);
            onOpen(id);
          }}
        />
      )}

      {renaming && (
        <RenameProjectModal
          project={renaming}
          onClose={() => setRenaming(null)}
          onRenamed={() => {
            setRenaming(null);
            void refresh();
          }}
        />
      )}

      {deleting && (
        <DeleteProjectModal
          project={deleting}
          onClose={() => setDeleting(null)}
          onDeleted={() => {
            setDeleting(null);
            void refresh();
          }}
        />
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------

function NewProjectModal({ onClose, onCreated }: { onClose: () => void; onCreated: (id: string) => void }) {
  const [name, setName] = useState("Untitled");
  const [tempo, setTempo] = useState(String(DEFAULT_TEMPO));
  const [tsIndex, setTsIndex] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const tempoNumber = Number(tempo);
  const tempoValid = Number.isFinite(tempoNumber) && tempoNumber >= MIN_TEMPO && tempoNumber <= MAX_TEMPO;
  const nameValid = name.trim().length > 0;

  async function submit() {
    if (!nameValid || !tempoValid || busy) return;
    setBusy(true);
    setError(null);
    try {
      const manifest = await api.createProject(name, tempoNumber, TIME_SIGNATURES[tsIndex]!);
      logger.info(`Created project "${manifest.name}"`);
      onCreated(manifest.id);
    } catch (e) {
      setError(errorMessage(e));
      setBusy(false);
    }
  }

  return (
    <Modal
      title="New Project"
      onClose={onClose}
      footer={
        <>
          <button className="btn" onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button className="btn btn--primary" onClick={submit} disabled={!nameValid || !tempoValid || busy}>
            {busy ? "Creating…" : "Create"}
          </button>
        </>
      }
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
        className="picker__form"
      >
        <label className="field">
          <span className="field__label">Name</span>
          <input
            className={`input ${name && !nameValid ? "input--invalid" : ""}`}
            value={name}
            onChange={(e) => setName(e.target.value)}
            autoFocus
            onFocus={(e) => e.target.select()}
          />
        </label>

        <div className="picker__form-row">
          <label className="field">
            <span className="field__label">Tempo</span>
            <input
              className={`input ${tempo && !tempoValid ? "input--invalid" : ""}`}
              value={tempo}
              inputMode="decimal"
              onChange={(e) => setTempo(e.target.value)}
            />
            {tempo && !tempoValid && (
              <span className="field__error">
                Must be between {MIN_TEMPO} and {MAX_TEMPO}
              </span>
            )}
          </label>

          <label className="field">
            <span className="field__label">Time Signature</span>
            <select className="input" value={tsIndex} onChange={(e) => setTsIndex(Number(e.target.value))}>
              {TIME_SIGNATURES.map((ts, i) => (
                <option key={`${ts.numerator}/${ts.denominator}`} value={i}>
                  {ts.numerator}/{ts.denominator}
                </option>
              ))}
            </select>
          </label>
        </div>

        <p className="field__hint">
          Tempo and time signature can be changed later from the Track Inspector.
        </p>

        {error && <p className="field__error">{error}</p>}
        <button type="submit" className="sr-only" tabIndex={-1} aria-hidden="true" />
      </form>
    </Modal>
  );
}

// ---------------------------------------------------------------------------

function RenameProjectModal({
  project,
  onClose,
  onRenamed,
}: {
  project: ProjectSummary;
  onClose: () => void;
  onRenamed: () => void;
}) {
  const [name, setName] = useState(project.name);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const valid = name.trim().length > 0 && name.trim() !== project.name;

  async function submit() {
    if (!valid || busy) return;
    setBusy(true);
    setError(null);
    try {
      await api.renameProject(project.id, name);
      logger.info(`Renamed "${project.name}" to "${name.trim()}"`);
      onRenamed();
    } catch (e) {
      setError(errorMessage(e));
      setBusy(false);
    }
  }

  return (
    <Modal
      title="Rename Project"
      onClose={onClose}
      footer={
        <>
          <button className="btn" onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button className="btn btn--primary" onClick={submit} disabled={!valid || busy}>
            Rename
          </button>
        </>
      }
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <label className="field">
          <span className="field__label">Name</span>
          <input
            className="input"
            value={name}
            onChange={(e) => setName(e.target.value)}
            autoFocus
            onFocus={(e) => e.target.select()}
          />
        </label>
        <p className="field__hint" style={{ marginTop: "var(--space-2)" }}>
          The project folder stays at <span className="mono">{project.id}</span>; only the display
          name changes.
        </p>
        {error && <p className="field__error">{error}</p>}
        <button type="submit" className="sr-only" tabIndex={-1} aria-hidden="true" />
      </form>
    </Modal>
  );
}

// ---------------------------------------------------------------------------

function DeleteProjectModal({
  project,
  onClose,
  onDeleted,
}: {
  project: ProjectSummary;
  onClose: () => void;
  onDeleted: () => void;
}) {
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function confirm() {
    setBusy(true);
    setError(null);
    try {
      await api.deleteProject(project.id);
      logger.info(`Deleted project "${project.name}"`);
      onDeleted();
    } catch (e) {
      setError(errorMessage(e));
      setBusy(false);
    }
  }

  return (
    <Modal
      title="Delete Project"
      onClose={onClose}
      footer={
        <>
          <button className="btn" onClick={onClose} disabled={busy}>
            Cancel
          </button>
          <button className="btn btn--danger" onClick={confirm} disabled={busy}>
            {busy ? "Deleting…" : "Delete"}
          </button>
        </>
      }
    >
      <p>
        Delete <strong>{project.name}</strong> and all{" "}
        {project.track_count === 1 ? "its track" : `${project.track_count} of its tracks`}?
      </p>
      <p className="field__hint">
        This removes the project folder from disk and cannot be undone.
      </p>
      {error && <p className="field__error">{error}</p>}
    </Modal>
  );
}
