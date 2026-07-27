import { useEffect, useRef } from "react";

interface Shortcuts {
  playing: boolean;
  play: () => void | Promise<void>;
  stop: () => void | Promise<void>;
  undo: () => void | Promise<void>;
  redo: () => void | Promise<void>;
  save: () => void | Promise<void>;
  toggleRecord: () => void | Promise<void>;
  toggleListen: () => void | Promise<void>;
}

/**
 * The shortcuts that work anywhere in the editor, as opposed to the ones the piano roll
 * owns while it has focus.
 *
 * Everything that changes on every keystroke is held in a ref, so the listener is bound
 * once rather than torn down and re-added as state moves.
 */
export function useEditorShortcuts({
  playing,
  play,
  stop,
  undo,
  redo,
  save,
  toggleRecord,
  toggleListen,
}: Shortcuts) {
  const playingRef = useRef(playing);
  playingRef.current = playing;
  const toggleRecordRef = useRef(toggleRecord);
  toggleRecordRef.current = toggleRecord;
  const toggleListenRef = useRef(toggleListen);
  toggleListenRef.current = toggleListen;

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      const target = event.target as HTMLElement | null;
      if (
        target &&
        (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable)
      ) {
        return;
      }

      const mod = event.metaKey || event.ctrlKey;

      if (event.code === "Space") {
        event.preventDefault();
        void (playingRef.current ? stop() : play());
        return;
      }

      if (mod && event.key.toLowerCase() === "z") {
        event.preventDefault();
        void (event.shiftKey ? redo() : undo());
        return;
      }

      if (mod && event.key.toLowerCase() === "s") {
        event.preventDefault();
        void save();
        return;
      }

      if (!mod && event.key.toLowerCase() === "r") {
        event.preventDefault();
        void toggleRecordRef.current();
        return;
      }

      // Listen sits next to Record on the keyboard as it does in the transport, because
      // it is the peer of Record in this app rather than something behind a panel.
      if (!mod && event.key.toLowerCase() === "l") {
        event.preventDefault();
        void toggleListenRef.current();
      }
    }

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [play, stop, undo, redo, save]);
}
