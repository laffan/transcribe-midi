/**
 * The shortcut reference in the inspector.
 *
 * Its own file because it is a table of facts, not logic, and it has to be kept in step
 * with three other places: the global handler in `useEditorShortcuts`, the piano roll's own
 * handler, and the list in README-TECHNICAL.md that a manual test reads from.
 */
export function ShortcutList() {
  return (
    <details className="inspector__shortcuts">
      <summary className="field__label">Shortcuts</summary>
      <dl className="shortcuts">
        <dt className="mono">Space</dt>
        <dd>Play / stop</dd>
        <dt className="mono">⌘Z / ⇧⌘Z</dt>
        <dd>Undo / redo</dd>
        <dt className="mono">⌘A</dt>
        <dd>Select all</dd>
        <dt className="mono">⌘C / ⌘X / ⌘V</dt>
        <dd>Copy / cut / paste</dd>
        <dt className="mono">⌘Q</dt>
        <dd>Quantize selection</dd>
        <dt className="mono">⌫</dt>
        <dd>Delete selection</dd>
        <dt className="mono">↑ ↓ ← →</dt>
        <dd>Nudge (⇧ for octave / bar)</dd>
        <dt className="mono">⌥click</dt>
        <dd>Delete note</dd>
        <dt className="mono">A–L, W/E/T/Y/U</dt>
        <dd>Play keys</dd>
        <dt className="mono">Z / X</dt>
        <dd>Octave down / up</dd>
        <dt className="mono">L</dt>
        <dd>Listen (audio → MIDI)</dd>
        <dt className="mono">⌘K</dt>
        <dd>Focus the prompt</dd>
      </dl>
    </details>
  );
}
