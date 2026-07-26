import type { AiProposal, NoteDiff, TranscriptionPreview } from "../../lib/types";

/**
 * Something that produced notes and is waiting on a decision.
 *
 * The two ways this app makes notes — describing an edit, and transcribing audio — are
 * the same interaction with different innards: produce, review on the roll, accept or
 * reject. Modelling that once means one review bar, one preview overlay on the piano
 * roll, and one rule that the roll is read-only while a decision is outstanding.
 */
export type Pending =
  | { kind: "ai"; proposal: AiProposal }
  | { kind: "transcription"; preview: TranscriptionPreview }
  | null;

/** What the piano roll should draw over the current notes. */
export function previewDiff(pending: Pending): NoteDiff | null {
  if (!pending) return null;
  if (pending.kind === "ai") return pending.proposal.empty ? null : pending.proposal.diff;

  // A transcription is pure addition: it never removes or moves an existing note.
  const added = pending.preview.notes.map((detected) => detected.note);
  return added.length > 0 ? { added, removed: [], changed: [] } : null;
}
