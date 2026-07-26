import { useCallback, useEffect, useState } from "react";

import { Modal } from "../../components/Modal";
import { api, errorMessage, isTauri } from "../../lib/api";
import { logger } from "../../lib/console";
import type { AiStatus, BuildInfo, InputSettings, ModelInfo } from "../../lib/types";
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
  const [settings, setSettings] = useState<InputSettings | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setSettings(await api.inputSettings());
    } catch (error) {
      logger.error("Could not read input settings", errorMessage(error));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  async function connect(portId: string) {
    setBusy(true);
    try {
      if (portId === "") {
        await api.midiDisconnect();
        logger.info("MIDI input disconnected");
      } else {
        const port = await api.midiConnect(portId);
        logger.info(`Connected to ${port.name}`);
      }
      await refresh();
    } catch (error) {
      logger.error("Could not change the MIDI port", errorMessage(error));
    } finally {
      setBusy(false);
    }
  }

  if (!settings) {
    return <p className="muted">Loading…</p>;
  }

  return (
    <div className="settings__group">
      <label className="field">
        <span className="field__label">MIDI input</span>
        <select
          className="input"
          value={settings.connected?.id ?? ""}
          disabled={busy}
          onChange={(e) => void connect(e.target.value)}
        >
          <option value="">None</option>
          {settings.ports.map((port) => (
            <option key={port.id} value={port.id}>
              {port.name}
            </option>
          ))}
        </select>
        {settings.ports.length === 0 && (
          <span className="field__hint">
            No MIDI inputs found. Connect a controller and reopen this panel.
          </span>
        )}
      </label>

      <label className="field">
        <span className="field__label">Input channel</span>
        <select
          className="input"
          value={settings.channel ?? ""}
          onChange={(e) => {
            const value = e.target.value === "" ? null : Number(e.target.value);
            setSettings({ ...settings, channel: value });
            api.midiSetChannel(value).catch((error) =>
              logger.error("Could not set the channel filter", errorMessage(error)),
            );
          }}
        >
          <option value="">All channels</option>
          {Array.from({ length: 16 }, (_, i) => (
            <option key={i} value={i}>
              Channel {i + 1}
            </option>
          ))}
        </select>
        <span className="field__hint">
          Most controllers send on channel 1, but not all — leave this on All unless
          something is filtering incorrectly.
        </span>
      </label>

      <label className="field">
        <span className="field__label">Keyboard velocity</span>
        <input
          className="input"
          type="range"
          min={1}
          max={127}
          value={settings.keyboard_velocity}
          onChange={(e) => {
            const velocity = Number(e.target.value);
            setSettings({ ...settings, keyboard_velocity: velocity });
            api.setKeyboardVelocity(velocity).catch(() => {});
          }}
        />
        <span className="field__hint mono">{settings.keyboard_velocity}</span>
      </label>

      <label className="field">
        <span className="field__label">Count-in</span>
        <select
          className="input"
          value={settings.count_in_bars}
          onChange={(e) => {
            const bars = Number(e.target.value);
            setSettings({ ...settings, count_in_bars: bars });
            api.setCountInBars(bars).catch(() => {});
          }}
        >
          <option value={0}>Off</option>
          <option value={1}>1 bar</option>
          <option value={2}>2 bars</option>
          <option value={4}>4 bars</option>
        </select>
        <span className="field__hint">
          Only the click sounds during the lead-in; recording arms at the playhead.
        </span>
      </label>

      <label className="field">
        <span className="field__label">Metronome</span>
        <select
          className="input"
          value={settings.metronome ? "on" : "off"}
          onChange={(e) => {
            const enabled = e.target.value === "on";
            setSettings({ ...settings, metronome: enabled });
            api.setMetronome(enabled).catch(() => {});
          }}
        >
          <option value="off">Off</option>
          <option value="on">On</option>
        </select>
      </label>

      <Pending phase="Phase 9" items={["Audio output device (macOS)"]} />
    </div>
  );
}

function AiTab() {
  const [status, setStatus] = useState<AiStatus | null>(null);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [draftKey, setDraftKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const next = await api.aiStatus();
      setStatus(next);
      return next;
    } catch (e) {
      setError(errorMessage(e));
      return null;
    }
  }, []);

  useEffect(() => {
    void (async () => {
      const next = await refresh();
      // Only reach for the network when there is a key to authenticate with; otherwise
      // opening this tab would produce a pointless failure every time.
      if (!next?.has_key) return;
      try {
        const listed = await api.aiModels();
        setModels(listed.models);
      } catch (e) {
        setError(errorMessage(e));
      }
    })();
  }, [refresh]);

  async function saveKey() {
    if (!draftKey.trim()) return;
    setBusy(true);
    setError(null);
    try {
      // Rust verifies the key against the API before storing it, and returns the model
      // list from the same call — so a typo is caught here rather than mid-edit.
      const listed = await api.aiSetKey(draftKey);
      setModels(listed.models);
      setDraftKey("");
      await refresh();
      logger.info(`API key saved — ${listed.models.length} models available`);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  async function removeKey() {
    setBusy(true);
    try {
      setStatus(await api.aiClearKey());
      setModels([]);
      logger.info("API key removed from the Keychain");
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="settings__group">
      <label className="field">
        <span className="field__label">Anthropic API key</span>
        {status?.has_key ? (
          <div className="settings__row">
            <span className="mono">{status.key_hint ?? "stored"}</span>
            <button className="btn btn--danger" disabled={busy} onClick={() => void removeKey()}>
              Remove
            </button>
          </div>
        ) : (
          <div className="settings__row">
            <input
              className="input"
              type="password"
              autoComplete="off"
              spellCheck={false}
              placeholder="sk-ant-…"
              value={draftKey}
              disabled={busy}
              onChange={(e) => setDraftKey(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  void saveKey();
                }
              }}
            />
            <button
              className="btn btn--primary"
              disabled={busy || draftKey.trim().length === 0}
              onClick={() => void saveKey()}
            >
              {busy ? "Checking…" : "Save"}
            </button>
          </div>
        )}
        <span className="field__hint">
          Stored in the platform Keychain — never in <span className="mono">project.json</span>,
          never in localStorage, and never handed to the interface. Every request to Anthropic is
          made by Rust. Once saved, the key cannot be read back here; only its last four
          characters are shown.
        </span>
        {status && !status.key_persists && (
          <span className="field__hint">
            This build has no Keychain, so the key is held in memory and is forgotten when the app
            quits. Use the macOS or iOS build for anything real.
          </span>
        )}
      </label>

      {error && <p className="settings__error">{error}</p>}

      <label className="field">
        <span className="field__label">Model</span>
        <select
          className="input"
          value={status?.model ?? ""}
          disabled={!status?.has_key || models.length === 0}
          onChange={(e) => {
            const id = e.target.value;
            api
              .aiSetModel(id)
              .then(setStatus)
              .catch((err) => setError(errorMessage(err)));
          }}
        >
          {models.length === 0 && <option value="">{status?.model ?? "No models loaded"}</option>}
          {models.map((model) => (
            <option key={model.id} value={model.id}>
              {model.display_name || model.id}
            </option>
          ))}
        </select>
        <span className="field__hint">
          Fetched from <span className="mono">GET /v1/models</span> when the key is saved, rather
          than hardcoded — a baked-in list is wrong the week a model ships. The default is the
          newest Sonnet-class model the key can reach.
        </span>
      </label>
    </div>
  );
}

function AboutTab() {
  const [root, setRoot] = useState<string | null>(null);
  const [build, setBuild] = useState<BuildInfo | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .projectsRoot()
      .then(setRoot)
      .catch((e) => setError(errorMessage(e)));
    api.buildInfo().then(setBuild).catch(() => setBuild(null));
  }, []);

  return (
    <div className="settings__group">
      <div className="field">
        <span className="field__label">Build</span>
        {/* Stamped at compile time, not read from disk. When the plugin and the app
            disagree about which build is running, this is the side that cannot lie. */}
        <div className="mono">
          {build
            ? `${build.version} · ${build.commit}${build.dirty ? " (modified)" : ""} · ${build.profile}`
            : "…"}
        </div>
        {build?.dirty && (
          <span className="field__hint">
            Built from a working tree with uncommitted changes, so the commit above
            identifies the last commit rather than exactly this code.
          </span>
        )}
        {build?.built_at && <span className="field__hint mono">built {build.built_at}</span>}
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
