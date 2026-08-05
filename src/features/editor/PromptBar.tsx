import { useCallback, useEffect, useRef, useState } from "react";

import { api, errorMessage, isCommandError } from "../../lib/api";
import { logger } from "../../lib/console";
import type { AiProposal, AiStatus, AiTarget } from "../../lib/types";
import "./PromptBar.css";

interface PromptBarProps {
  trackIndex: number;
  trackName: string;
  selection: number[];
  /** Changes when Settings closes, so the key and model are re-read. */
  settingsRevision: number;
  /** Suppressed while a proposal is on screen — one decision at a time. */
  disabled: boolean;
  onProposal: (proposal: AiProposal) => void;
  onOpenSettings: () => void;
}

/**
 * The prompt bar: describe an edit in words.
 *
 * Spans the editor above the transport rather than sitting in the inspector, because
 * describing a change is the primary way to make one in this app and a panel you have to
 * scroll to does not read that way. `⌘K` focuses it from anywhere.
 *
 * Nothing here talks to Anthropic. The prompt goes to Rust, Rust runs the tool loop and
 * holds the resulting transaction, and this gets back a diff. The API key never enters
 * the webview and the transaction never leaves Rust.
 */
export function PromptBar({
  trackIndex,
  trackName,
  selection,
  settingsRevision,
  disabled,
  onProposal,
  onOpenSettings,
}: PromptBarProps) {
  const [status, setStatus] = useState<AiStatus | null>(null);
  const [prompt, setPrompt] = useState("");
  const [target, setTarget] = useState<AiTarget>("this_track");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    api.aiStatus().then(setStatus).catch(() => setStatus(null));
  }, [settingsRevision]);

  // ⌘K from anywhere. The one shortcut that has to work while a text field elsewhere has
  // focus, because its whole job is to take focus.
  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        inputRef.current?.focus();
        inputRef.current?.select();
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  const submit = useCallback(async () => {
    const text = prompt.trim();
    if (!text || busy || disabled) return;

    setBusy(true);
    setError(null);
    try {
      const proposal = await api.aiPropose(trackIndex, text, selection, target);
      onProposal(proposal);
      if (proposal.empty) {
        logger.info(proposal.narration || "The model made no changes");
      } else {
        logger.info(`AI proposal: ${proposal.summary}`, proposal.narration);
      }
      if (proposal.truncated) {
        logger.warn("The model ran out of turns — the proposal may be incomplete");
      }
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      if (!(isCommandError(e) && (e.code === "no_api_key" || e.code === "no_model"))) {
        logger.error("The AI edit failed", message);
      }
    } finally {
      setBusy(false);
    }
  }, [prompt, busy, disabled, trackIndex, selection, target, onProposal]);

  const needsSetup = status !== null && (!status.has_key || !status.model);

  if (needsSetup) {
    return (
      <div className="promptbar promptbar--setup">
        {/* One line. Where the key is kept and who makes the request is a real
            reassurance, but it is three lines of prose in a control bar and it is
            already said in Settings, next to the field you type the key into. */}
        <span className="promptbar__hint">Add an API key to describe edits in words.</span>
        <button className="btn" onClick={onOpenSettings}>
          Set up AI
        </button>
      </div>
    );
  }

  return (
    <div className="promptbar">
      <label className="promptbar__scope">
        <select
          className="input promptbar__target"
          value={target}
          disabled={busy || disabled}
          onChange={(e) => setTarget(e.target.value as AiTarget)}
          aria-label="Where the edit goes"
        >
          <option value="this_track">Edit {trackName}</option>
          <option value="new_track">Write a new part</option>
        </select>
      </label>

      <textarea
        ref={inputRef}
        className="input promptbar__input"
        rows={1}
        placeholder={
          target === "new_track"
            ? "Describe a part to write against this one — “a walking bass line”…"
            : selection.length > 0
              ? `Describe an edit to the ${selection.length} selected note${selection.length === 1 ? "" : "s"}…`
              : "Describe an edit — “make it swing”, “harmonise a third above”…"
        }
        value={prompt}
        disabled={busy || disabled}
        onChange={(e) => setPrompt(e.target.value)}
        onKeyDown={(e) => {
          // Return submits, ⇧Return is a newline. The opposite of the Phase 6 panel:
          // once this is the primary input, one line is the common case and reaching for
          // a modifier every time is friction on the main path.
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            void submit();
          }
        }}
      />

      <button
        className="btn btn--primary"
        onClick={() => void submit()}
        disabled={busy || disabled || prompt.trim().length === 0}
      >
        {busy ? "Thinking…" : "Suggest"}
      </button>

      {error && <span className="promptbar__error">{error}</span>}
      {!error && !busy && (
        <span className="promptbar__hint mono">
          {selection.length > 0 ? `${selection.length} selected` : "whole track"} · ⌘K
        </span>
      )}
    </div>
  );
}
