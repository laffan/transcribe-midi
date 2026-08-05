import { useEffect, useState } from "react";

/**
 * Subscribes to a CSS media query from React.
 *
 * Layout belongs in CSS and stays there. This exists for the cases where the *markup*
 * has to differ rather than its presentation — where no amount of styling fixes it
 * because the elements themselves are wrong for the device. Reach for a media query in
 * a stylesheet first; reach for this only when you would otherwise be rendering
 * something and then hiding it.
 *
 * The query string is the same text a stylesheet would use, so the rungs in
 * `styles/tokens.css` are the vocabulary here too.
 */
export function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState(() => {
    // `matchMedia` is absent in a non-browser environment (a test runner, SSR). False
    // is the honest answer there: it is the desktop layout, which is the wider one.
    if (typeof window === "undefined" || !window.matchMedia) return false;
    return window.matchMedia(query).matches;
  });

  useEffect(() => {
    if (typeof window === "undefined" || !window.matchMedia) return;
    const list = window.matchMedia(query);

    // Read once on subscribe as well as on change: between the initial state above and
    // this effect running, the query can already have flipped — a phone rotated during
    // the first paint would otherwise keep the layout it started in until the *next*
    // rotation.
    setMatches(list.matches);

    const onChange = (event: MediaQueryListEvent) => setMatches(event.matches);
    list.addEventListener("change", onChange);
    return () => list.removeEventListener("change", onChange);
  }, [query]);

  return matches;
}

/**
 * The phone-portrait rung of the breakpoint ladder, as one importable constant so a
 * component and a stylesheet cannot drift to different numbers. See `styles/tokens.css`
 * for why 430px is the line.
 */
export const PHONE_PORTRAIT = "(max-width: 430px)";

/** The phone-landscape rung. Paired with a coarse pointer so a short desktop window,
 *  which has a cursor and plenty of width, is not mistaken for a phone on its side. */
export const PHONE_LANDSCAPE = "(max-height: 460px) and (pointer: coarse)";

/**
 * Either phone rung — a phone, whichever way up it is held.
 *
 * One comma-joined query rather than two calls to `useMediaQuery`, because `||` between
 * two hook calls short-circuits the second one and React counts hooks by call order.
 */
export const PHONE = `${PHONE_PORTRAIT}, ${PHONE_LANDSCAPE}`;
