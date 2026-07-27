/**
 * The design tokens the roll paints with, resolved once per frame.
 *
 * A canvas cannot use CSS custom properties, so the values have to be read out of the
 * document and passed in as strings. Reading them per frame rather than caching is what
 * makes the roll follow a theme change without a remount; each fallback matches the value
 * in `src/styles/tokens.css` so a missing token degrades to the intended colour rather than
 * to transparent black.
 */
export interface RollTheme {
  bgInset: string;
  bg1: string;
  gridLine: string;
  gridBar: string;
  border: string;
  text2: string;
  accent: string;
  playhead: string;
  keyWhite: string;
  keyBlack: string;
  diffAdded: string;
  diffRemoved: string;
  diffChanged: string;
}

export function readRollTheme(): RollTheme {
  const style = getComputedStyle(document.documentElement);
  const token = (name: string, fallback: string) =>
    style.getPropertyValue(name).trim() || fallback;

  return {
    bgInset: token("--bg-inset", "#0a0b0e"),
    bg1: token("--bg-1", "#15171c"),
    gridLine: token("--grid-line", "#21252e"),
    gridBar: token("--grid-line-bar", "#333a48"),
    border: token("--border", "#2b303b"),
    text2: token("--text-2", "#6f7689"),
    accent: token("--accent", "#5b8dd9"),
    playhead: token("--playhead", "#d9a441"),
    keyWhite: token("--key-white", "#e8eaf0"),
    keyBlack: token("--key-black", "#1c1f26"),
    diffAdded: token("--diff-added", "#6cb08a"),
    diffRemoved: token("--diff-removed", "#d16b6b"),
    diffChanged: token("--diff-changed", "#d9a441"),
  };
}
