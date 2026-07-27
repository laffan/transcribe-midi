import { useCallback, useEffect, useMemo, useState } from "react";

import { api, errorMessage } from "../../lib/api";
import { logger } from "../../lib/console";
import type { EditorState, EditRequest, ProjectManifest } from "../../lib/types";

/**
 * The open project: what it holds, what is selected in it, and every way to change it.
 *
 * Selection lives here rather than beside the piano roll because the backend decides it.
 * Rust reports where edited notes ended up — indices shift whenever notes reorder — so
 * after any mutation the selection is re-derived from that report, and a copy kept next to
 * the view would drift onto different notes.
 */
export function useEditorSession(projectId: string) {
  const [manifest, setManifest] = useState<ProjectManifest | null>(null);
  const [editor, setEditor] = useState<EditorState | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [selectedTrack, setSelectedTrack] = useState(0);
  const [selection, setSelection] = useState<number[]>([]);
  const [tempo, setTempo] = useState(120);

  useEffect(() => {
    let cancelled = false;

    (async () => {
      try {
        const [project, state] = await Promise.all([
          api.loadProject(projectId),
          api.openProject(projectId),
        ]);
        if (cancelled) return;
        setManifest(project.manifest);
        setTempo(project.manifest.tempo_bpm);
        setEditor(state);
        setLoadError(null);
      } catch (error) {
        if (cancelled) return;
        const message = errorMessage(error);
        setLoadError(message);
        logger.error(`Could not open project "${projectId}"`, message);
      }
    })();

    return () => {
      cancelled = true;
      void api.closeProject().catch(() => {});
    };
  }, [projectId]);

  const applyEdit = useCallback(async (request: EditRequest) => {
    try {
      const state = await api.applyEdit(request);
      setEditor(state);
      // Rust reports where the edited notes ended up; indices shift when notes reorder,
      // so the selection is re-derived from that rather than kept locally.
      if (state.affected.length > 0) setSelection(state.affected);
    } catch (error) {
      logger.error("Edit failed", errorMessage(error));
    }
  }, []);

  const undo = useCallback(async () => {
    try {
      setEditor(await api.undo());
      setSelection([]);
    } catch (error) {
      logger.error("Undo failed", errorMessage(error));
    }
  }, []);

  const redo = useCallback(async () => {
    try {
      setEditor(await api.redo());
      setSelection([]);
    } catch (error) {
      logger.error("Redo failed", errorMessage(error));
    }
  }, []);

  const save = useCallback(async () => {
    try {
      setEditor(await api.saveOpenProject());
      logger.info("Project saved");
    } catch (error) {
      logger.error("Save failed", errorMessage(error));
    }
  }, []);

  const selectTrack = useCallback((index: number) => {
    setSelectedTrack(index);
    setSelection([]);
  }, []);

  const changeTempo = useCallback((bpm: number) => {
    setTempo(bpm);
    void api
      .setTempo(bpm)
      .catch((error) => logger.error("Tempo change failed", errorMessage(error)));
  }, []);

  const addTrack = useCallback(async () => {
    if (!manifest) return;
    try {
      await api.addTrack(manifest.id);
      const state = await api.openProject(manifest.id);
      setEditor(state);
      setSelectedTrack(state.tracks.length - 1);
      setSelection([]);
      logger.info("Added track");
    } catch (error) {
      logger.error("Could not add track", errorMessage(error));
    }
  }, [manifest]);

  const deleteTrack = useCallback(
    async (trackId: string) => {
      if (!manifest || !editor) return;
      if (editor.tracks.length <= 1) {
        logger.warn("A project must keep at least one track");
        return;
      }
      if (editor.dirty) {
        logger.warn("Save before removing a track — unsaved edits would be lost");
        return;
      }
      try {
        await api.deleteTrack(manifest.id, trackId);
        const state = await api.openProject(manifest.id);
        setEditor(state);
        setSelectedTrack(0);
        setSelection([]);
        logger.info("Deleted track");
      } catch (error) {
        logger.error("Could not delete track", errorMessage(error));
      }
    },
    [manifest, editor],
  );

  const track = useMemo(() => editor?.tracks[selectedTrack] ?? null, [editor, selectedTrack]);

  return {
    manifest,
    editor,
    setEditor,
    loadError,
    track,
    selectedTrack,
    setSelectedTrack,
    selectTrack,
    selection,
    setSelection,
    tempo,
    changeTempo,
    applyEdit,
    undo,
    redo,
    save,
    addTrack,
    deleteTrack,
  };
}

export type EditorSession = ReturnType<typeof useEditorSession>;
