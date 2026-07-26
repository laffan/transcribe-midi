import { useCallback, useEffect, useRef, useState } from "react";

import { api, errorMessage, isCommandError } from "../../lib/api";
import { logger } from "../../lib/console";
import type { AiProposal, AiStatus, EditorState } from "../../lib/types";
import "./AiPanel.css";

interface AiPanelProps {
  trackIndex: number;
  selection: number[];
  /** Changes when Settings closes, so the key and model are re-read. */
  settingsRevision: number;
  onApplied: (state: EditorState) => void;
  onPreviewChange: (proposal: AiProposal | null) => void;
  onOpenSettings: () => void;
}

/**
 * Phase 6: describe an edit, see it as a diff, accept or reject.
 *
 * Nothing here talks to Anthropic. The prompt goes to Rust, Rust runs the tool loop and
 * holds the resulting transaction, and this component gets back a diff to draw. That is
 * not incidental — the API key never enters the webview, and the transaction never
 * leaves Rust, so there is no path from this file to a note.
 */
export function AiPanel({
  trackIndex,
  selection,
  settingsRevision,
  onApplied,
  onPreviewChange,
  onOpenSettings,
}: AiPanelProps) {
  const [status, setStatus] = useState<AiStatus | null>(null);
  const [prompt, setPrompt] = useState("");
  const [busy, setBusy] = useState(false);
  const [proposal, setProposal] = useState<AiProposal | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showSteps, setShowSteps] = useState(false);

  const onPreviewChangeRef = useRef(onPreviewChange);
  onPreviewChangeRef.current = onPreviewChange;

  useEffect(() => {
    api.aiStatus().then(setStatus).catch(() => setStatus(null));
  }, [settingsRevision]);

  const clearProposal = useCallback(() => {
    setProposal(null);
    onPreviewChangeRef.current(null);
  }, []);

  // A proposal is bound to one track's note indices. Switching tracks makes it
  // meaningless, so it goes rather than sitting there looking valid.
  useEffect(() => {
    setProposal((current) => {
      if (!current) return null;
      void api.aiReject().catch(() => {});
      onPreviewChangeRef.current(null);
      return null;
    });
  }, [trackIndex]);

  async function submit() {
    const text = prompt.trim();
    if (!text || busy) return;

    setBusy(true);
    setError(null);
    clearProposal();

    try {
      const result = await api.aiPropose(trackIndex, text, selection);
      setProposal(result);
      onPreviewChangeRef.current(result.empty ? null : result);

      if (result.empty) {
        logger.info(result.narration || "The model made no changes");
      } else {
        logger.info(`AI proposal: ${result.summary}`, result.narration);
      }
      if (result.truncated) {
        logger.warn("The model ran out of turns — the proposal may be incomplete");
      }
    } catch (e) {
      const message = errorMessage(e);
      setError(message);
      if (!(isCommandError(e) && e.code === "no_api_key")) {
        logger.error("The AI edit failed", message);
      }
    } finally {
      setBusy(false);
    }
  }

  async function accept() {
    try {
      const state = await api.aiAccept();
      onApplied(state);
      logger.info(`Applied: ${proposal?.summary ?? "AI edit"} — undo it like any other edit`);
      clearProposal();
      setPrompt("");
    } catch (e) {
      setError(errorMessage(e));
      logger.error("Could not apply the suggestion", errorMessage(e));
    }
  }

  async function reject() {
    await api.aiReject().catch(() => {});
    clearProposal();
    logger.info("Discarded the suggestion");
  }

  const noKey = status !== null && !status.has_key;
  const noModel = status !== null && status.has_key && !status.model;

  if (noKey || noModel) {
    return (
      <div className="ai">
        <div className="ai__head">
          <h3 className="ai__title">AI edit</h3>
        </div>
        <p className="field__hint">
          {noKey
            ? "Add an Anthropic API key to describe edits in words. It is stored in the Keychain and every request is made by Rust — the key is never handed to the interface."
            : "Choose a model to use AI editing."}
        </p>
        <button className="btn" onClick={onOpenSettings}>
          Open Settings → AI
        </button>
      </div>
    );
  }

  return (
    <div className="ai">
      <div className="ai__head">
        <h3 className="ai__title">AI edit</h3>
        {status?.model && <span className="ai__model mono truncate">{status.model}</span>}
      </div>

      <textarea
        className="input ai__prompt"
        rows={3}
        placeholder={
          selection.length > 0
            ? `Describe an edit to the ${selection.length} selected note${selection.length === 1 ? "" : "s"}…`
            : "Describe an edit to this track…"
        }
        value={prompt}
        disabled={busy}
        onChange={(e) => setPrompt(e.target.value)}
        onKeyDown={(e) => {
          // ⌘↩ submits. Plain Return stays a newline — these prompts run to a sentence
          // or two and losing one to a stray keystroke is worse than an extra chord.
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            void submit();
          }
        }}
      />

      <div className="ai__actions">
        <button
          className="btn btn--primary"
          onClick={() => void submit()}
          disabled={busy || prompt.trim().length === 0}
        >
          {busy ? "Thinking…" : "Suggest"}
        </button>
        <span className="field__hint">
          {selection.length > 0 ? `${selection.length} selected` : "whole track"}
        </span>
      </div>

      {error && <p className="ai__error">{error}</p>}

      {proposal && (
        <div className="ai__proposal">
          {proposal.narration && <p className="ai__narration">{proposal.narration}</p>}

          {proposal.empty ? (
            <p className="field__hint">Nothing to apply.</p>
          ) : (
            <>
              <div className="ai__counts">
                {proposal.diff.added.length > 0 && (
                  <span className="ai__count ai__count--added">
                    +{proposal.diff.added.length} added
                  </span>
                )}
                {proposal.diff.removed.length > 0 && (
                  <span className="ai__count ai__count--removed">
                    −{proposal.diff.removed.length} removed
                  </span>
                )}
                {proposal.diff.changed.length > 0 && (
                  <span className="ai__count ai__count--changed">
                    {proposal.diff.changed.length} changed
                  </span>
                )}
              </div>

              <p className="field__hint">
                Shown on the roll: green is new, red is going, amber is moving. The roll is
                read-only until you decide.
              </p>

              <div className="ai__actions">
                <button className="btn btn--primary" onClick={() => void accept()}>
                  Apply
                </button>
                <button className="btn" onClick={() => void reject()}>
                  Discard
                </button>
              </div>
            </>
          )}

          {proposal.steps.length > 0 && (
            <div className="ai__steps">
              <button
                className="btn btn--ghost ai__steps-toggle"
                onClick={() => setShowSteps((v) => !v)}
                aria-expanded={showSteps}
              >
                {showSteps ? "Hide" : "Show"} {proposal.steps.length} step
                {proposal.steps.length === 1 ? "" : "s"}
              </button>

              {showSteps && (
                <ol className="ai__step-list">
                  {proposal.steps.map((step, index) => (
                    <li
                      key={index}
                      className={`ai__step ${step.ok ? "" : "ai__step--failed"}`}
                    >
                      <span className="mono">{step.tool}</span>
                      <span className="ai__step-result">{step.result}</span>
                    </li>
                  ))}
                </ol>
              )}
            </div>
          )}

          <p className="ai__usage mono">
            {proposal.usage.input_tokens.toLocaleString()} in ·{" "}
            {proposal.usage.output_tokens.toLocaleString()} out
          </p>
        </div>
      )}
    </div>
  );
}
