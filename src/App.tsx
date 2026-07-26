import { useEffect, useState } from "react";

import { Editor } from "./features/editor/Editor";
import { ProjectPicker } from "./features/projects/ProjectPicker";
import { SettingsModal } from "./features/settings/SettingsModal";
import { applyTheme, loadTheme, type Theme } from "./lib/theme";

/**
 * Two screens: the project picker is the launch screen, and opening a project swaps in
 * the editor. Deliberately not a router — there are exactly two states and no URLs to
 * speak of inside a webview.
 */
export function App() {
  const [openProjectId, setOpenProjectId] = useState<string | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  const [theme, setThemeState] = useState<Theme>(loadTheme);

  useEffect(() => {
    applyTheme(theme);
    if (theme !== "system") return;

    // Follow the OS while "Match system" is selected.
    const media = window.matchMedia("(prefers-color-scheme: light)");
    const onChange = () => applyTheme("system");
    media.addEventListener("change", onChange);
    return () => media.removeEventListener("change", onChange);
  }, [theme]);

  return (
    <>
      {openProjectId === null ? (
        <ProjectPicker onOpen={setOpenProjectId} onOpenSettings={() => setShowSettings(true)} />
      ) : (
        <Editor
          projectId={openProjectId}
          onClose={() => setOpenProjectId(null)}
          onOpenSettings={() => setShowSettings(true)}
        />
      )}

      {showSettings && (
        <SettingsModal
          theme={theme}
          onThemeChange={setThemeState}
          onClose={() => setShowSettings(false)}
        />
      )}
    </>
  );
}
