import { useEffect, useState } from "react";

import { Modal } from "../../components/Modal";
import { api, errorMessage, isTauri } from "../../lib/api";
import type { Theme } from "../../lib/theme";
import { setTheme } from "../../lib/theme";
import "./SettingsModal.css";

type Tab = "appearance" | "input" | "ai" | "about";

const TABS: { id: Tab; label: string }[] = [
  { id: "appearance", label: "Appearance" },
  { id: "input", label: "Input" },
  { id: "ai", label: "AI" },
  { id: "about", label: "About" },
];

interface SettingsModalProps {
  theme: Theme;
  onThemeChange: (theme: Theme) => void;
  onClose: () => void;
}

export function SettingsModal({ theme, onThemeChange, onClose }: SettingsModalProps) {
  const [tab, setTab] = useState<Tab>("appearance");

  return (
    <Modal
      title="Settings"
      onClose={onClose}
      wide
      footer={
        <button className="btn btn--primary" onClick={onClose}>
          Done
        </button>
      }
    >
      <div className="settings">
        <nav className="settings__tabs" role="tablist" aria-label="Settings sections">
          {TABS.map((t) => (
            <button
              key={t.id}
              role="tab"
              aria-selected={tab === t.id}
              className={`settings__tab ${tab === t.id ? "settings__tab--active" : ""}`}
              onClick={() => setTab(t.id)}
            >
              {t.label}
            </button>
          ))}
        </nav>

        <div className="settings__panel" role="tabpanel">
          {tab === "appearance" && <AppearanceTab theme={theme} onThemeChange={onThemeChange} />}
          {tab === "input" && <InputTab />}
          {tab === "ai" && <AiTab />}
          {tab === "about" && <AboutTab />}
        </div>
      </div>
    </Modal>
  );
}

function AppearanceTab({ theme, onThemeChange }: { theme: Theme; onThemeChange: (t: Theme) => void }) {
  return (
    <div className="settings__group">
      <label className="field">
        <span className="field__label">Theme</span>
        <select
          className="input"
          value={theme}
          onChange={(e) => {
            const next = e.target.value as Theme;
            setTheme(next);
            onThemeChange(next);
          }}
        >
          <option value="dark">Dark</option>
          <option value="light">Light</option>
          <option value="system">Match system</option>
        </select>
        <span className="field__hint">Unplugged is designed dark-first; light is a courtesy.</span>
      </label>
    </div>
  );
}

function InputTab() {
  return (
    <div className="settings__group">
      <Pending
        phase="Phase 4"
        items={[
          "MIDI input port",
          "Input channel filter",
          "Metronome level and sound",
        ]}
      />
      <Pending phase="Phase 2" items={["On-screen keyboard velocity", "Audio output device (macOS)"]} />
    </div>
  );
}

function AiTab() {
  return (
    <div className="settings__group">
      <Pending phase="Phase 6" items={["Anthropic API key", "Model picker"]} />
      <p className="field__hint">
        The API key will be stored in the platform Keychain — never in{" "}
        <span className="mono">project.json</span>, never in localStorage, and never handed to the
        webview. Every request originates in Rust. The model list is fetched from{" "}
        <span className="mono">GET /v1/models</span> at runtime rather than hardcoded.
      </p>
    </div>
  );
}

function AboutTab() {
  const [root, setRoot] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .projectsRoot()
      .then(setRoot)
      .catch((e) => setError(errorMessage(e)));
  }, []);

  return (
    <div className="settings__group">
      <div className="field">
        <span className="field__label">Version</span>
        <div className="mono">0.1.0 — Phase 1</div>
      </div>

      <div className="field">
        <span className="field__label">Projects folder</span>
        <div className="mono settings__path">{error ?? root ?? "…"}</div>
      </div>

      {!isTauri() && (
        <p className="field__hint">
          Running as a browser preview. Projects are held in localStorage and the Rust backend is
          not involved — use the macOS or iOS build for anything real.
        </p>
      )}
    </div>
  );
}

function Pending({ phase, items }: { phase: string; items: string[] }) {
  return (
    <div className="settings__pending">
      <span className="placeholder__phase">{phase}</span>
      <ul className="settings__pending-list">
        {items.map((item) => (
          <li key={item}>{item}</li>
        ))}
      </ul>
    </div>
  );
}
