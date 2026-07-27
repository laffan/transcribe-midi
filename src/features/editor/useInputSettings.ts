import { useCallback, useEffect, useState } from "react";

import { api, errorMessage } from "../../lib/api";
import { logger } from "../../lib/console";

/**
 * Mirrors of the input settings Rust owns, re-read whenever Settings closes.
 *
 * These are display copies only — Rust applies the keyboard velocity and drives the click.
 * `toggleMetronome` flips the local copy first so the button responds immediately, and puts
 * it back if the call fails; a switch that lies about the click's state is worse than one
 * that is briefly slow.
 */
export function useInputSettings(settingsRevision: number) {
  const [keyboardVelocity, setKeyboardVelocity] = useState(100);
  const [metronome, setMetronome] = useState(false);

  useEffect(() => {
    api
      .inputSettings()
      .then((settings) => {
        setKeyboardVelocity(settings.keyboard_velocity);
        setMetronome(settings.metronome);
      })
      .catch(() => {});
  }, [settingsRevision]);

  const toggleMetronome = useCallback(async () => {
    const next = !metronome;
    setMetronome(next);
    try {
      await api.setMetronome(next);
    } catch (error) {
      setMetronome(!next);
      logger.error("Could not toggle the metronome", errorMessage(error));
    }
  }, [metronome]);

  return { keyboardVelocity, metronome, toggleMetronome };
}
