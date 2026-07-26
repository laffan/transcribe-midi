/**
 * Theme selection. Persisted to localStorage — this is a pure UI preference with no
 * security weight, so it does not need to make the trip to Rust.
 */

export type Theme = "dark" | "light" | "system";

const STORAGE_KEY = "unplugged.theme";

export function loadTheme(): Theme {
  const stored = localStorage.getItem(STORAGE_KEY);
  return stored === "light" || stored === "dark" || stored === "system" ? stored : "dark";
}

export function setTheme(theme: Theme): void {
  localStorage.setItem(STORAGE_KEY, theme);
  applyTheme(theme);
}

export function applyTheme(theme: Theme): void {
  const resolved =
    theme === "system"
      ? window.matchMedia("(prefers-color-scheme: light)").matches
        ? "light"
        : "dark"
      : theme;
  document.documentElement.dataset.theme = resolved;
}
