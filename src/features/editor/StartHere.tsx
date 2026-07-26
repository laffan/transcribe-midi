import "./StartHere.css";

interface StartHereProps {
  onListen: () => void;
  onDescribe: () => void;
}

/**
 * What an empty project offers instead of an empty grid.
 *
 * A blank piano roll and a mouse is how a MIDI editor introduces itself. This app is
 * about two other things, so those are the two doors — and this is the one place where
 * the app gets to say what it is before the user has to guess.
 *
 * It disappears the moment there is a note, by any route including drawing one.
 */
export function StartHere({ onListen, onDescribe }: StartHereProps) {
  return (
    <div className="starthere">
      <div className="starthere__doors">
        <button className="starthere__door" onClick={onListen}>
          <span className="starthere__verb">Play it</span>
          <span className="starthere__detail">
            Sing, hum or play a line — one note at a time — and it becomes notes.
          </span>
        </button>

        <button className="starthere__door" onClick={onDescribe}>
          <span className="starthere__verb">Describe it</span>
          <span className="starthere__detail">
            “A ii–V–I in C, arpeggiated in sixteenths.” You see a diff before anything
            is applied.
          </span>
        </button>
      </div>

      <p className="starthere__aside">
        Or just draw: click the grid behind this. Everything you make here is editable by
        hand — the two doors above are only the fastest way in.
      </p>
    </div>
  );
}
