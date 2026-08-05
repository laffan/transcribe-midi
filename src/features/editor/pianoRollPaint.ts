/**
 * Everything the roll draws, in the order it draws it.
 *
 * One function rather than several, because on a canvas the order *is* the logic: the
 * keyboard gutter is painted after the notes so notes scrolled off the left are covered
 * rather than clipped, and the playhead is last so it is never hidden. Splitting the
 * layers into separate files would let that ordering be changed by accident.
 *
 * Pure apart from the context it is handed — no component state, no refs, and no reads of
 * the DOM beyond the theme it is given.
 */

import type { Note, NoteDiff, TimeSignature, Track } from "../../lib/types";
import {
  isBlackKey,
  KEY_WIDTH,
  MAX_PITCH,
  MIN_PITCH,
  type Marquee,
  noteRect,
  normalizeMarquee,
  pitchName,
  pitchToY,
  RULER_HEIGHT,
  tickToX,
  type Viewport,
  xToTick,
} from "./pianoRollGeometry";
import type { RollTheme } from "./pianoRollTheme";

export interface RollScene {
  theme: RollTheme;
  width: number;
  height: number;
  /** Height of the note area. The velocity lane, when there is one, sits below it. */
  rollHeight: number;
  /** How tall that lane is, or 0 when the roll is too short to earn one. */
  velocityLane: number;
  view: Viewport;
  track: Track;
  selection: Set<number>;
  ppq: number;
  timeSignature: TimeSignature;
  /** Ticks between grid subdivisions, already resolved from the grid selector. */
  snap: number;
  playheadTicks: number;
  loopRegion: [number, number] | null;
  /** The rectangle being dragged out, or null when no marquee is in progress. */
  marquee: Marquee | null;
  preview: NoteDiff | null;
}

export function paintRoll(ctx: CanvasRenderingContext2D, scene: RollScene): void {
  const {
    theme,
    width,
    height,
    rollHeight,
    velocityLane,
    view,
    track,
    selection,
    ppq,
    timeSignature,
    snap,
    playheadTicks,
    loopRegion,
    marquee,
    preview,
  } = scene;
  const { pxPerTick, rowHeight, scrollTicks, topPitch } = view;

ctx.clearRect(0, 0, width, height);

    // ---- lane backgrounds (black-key rows sit darker, as on a real roll) ----
    ctx.fillStyle = theme.bgInset;
    ctx.fillRect(KEY_WIDTH, RULER_HEIGHT, width - KEY_WIDTH, rollHeight - RULER_HEIGHT);

    const firstPitch = Math.min(MAX_PITCH, topPitch);
    const lastPitch = Math.max(MIN_PITCH, topPitch - Math.ceil((rollHeight - RULER_HEIGHT) / rowHeight));

    for (let pitch = firstPitch; pitch >= lastPitch; pitch -= 1) {
      const y = pitchToY(pitch, view);
      if (y > rollHeight) continue;
      if (isBlackKey(pitch)) {
        ctx.fillStyle = "rgba(0,0,0,0.22)";
        ctx.fillRect(KEY_WIDTH, y, width - KEY_WIDTH, rowHeight);
      }
      // Octave boundaries read as the strongest horizontal rule.
      if (pitch % 12 === 0) {
        ctx.strokeStyle = theme.gridBar;
        ctx.beginPath();
        ctx.moveTo(KEY_WIDTH, y + rowHeight + 0.5);
        ctx.lineTo(width, y + rowHeight + 0.5);
        ctx.stroke();
      }
    }

    // ---- vertical grid ----
    const beatTicks = (ppq * 4) / timeSignature.denominator;
    const barTicks = beatTicks * timeSignature.numerator;
    const startTick = Math.max(0, scrollTicks);
    const endTick = xToTick(width, view);

    // Only draw subdivisions when they are far enough apart to be legible.
    const subdivision = snap > 0 && snap * pxPerTick >= 6 ? snap : beatTicks;

    ctx.lineWidth = 1;
    for (let tick = Math.floor(startTick / subdivision) * subdivision; tick <= endTick; tick += subdivision) {
      const x = Math.round(tickToX(tick, view)) + 0.5;
      if (x < KEY_WIDTH) continue;
      const isBar = tick % barTicks === 0;
      const isBeat = tick % beatTicks === 0;
      ctx.strokeStyle = isBar ? theme.gridBar : theme.gridLine;
      ctx.globalAlpha = isBar || isBeat ? 1 : 0.55;
      ctx.beginPath();
      ctx.moveTo(x, RULER_HEIGHT);
      ctx.lineTo(x, rollHeight);
      ctx.stroke();
    }
    ctx.globalAlpha = 1;

    // ---- notes ----
    track.notes.forEach((note, index) => {
      const rect = noteRect(note, index, view);
      if (rect.x + rect.width < KEY_WIDTH || rect.x > width) return;
      if (rect.y + rect.height < RULER_HEIGHT || rect.y > rollHeight) return;

      const selected = selection.has(index);
      // Velocity drives opacity so dynamics are legible at a glance. Under a preview
      // everything recedes so the proposed change is what the eye lands on.
      const alpha = (0.4 + (note.velocity / 127) * 0.6) * (preview ? 0.3 : 1);

      ctx.globalAlpha = alpha;
      ctx.fillStyle = selected && !preview ? theme.accent : track.color;
      const x = Math.max(KEY_WIDTH, rect.x);
      const drawn = rect.width - (x - rect.x);
      ctx.fillRect(x, rect.y + 1, Math.max(1, drawn), rect.height - 2);

      ctx.globalAlpha = 1;
      if (selected && !preview) {
        ctx.strokeStyle = theme.keyWhite;
        ctx.lineWidth = 1;
        ctx.strokeRect(x + 0.5, rect.y + 1.5, Math.max(1, drawn) - 1, rect.height - 3);
      }
    });
    ctx.globalAlpha = 1;

    // ---- proposed change ----
    //
    // Three colours, one meaning each: green is new, red is going, amber is moving.
    // Removed and "before" notes are outlined rather than filled, so a filled block
    // always means a note that will exist once the change is accepted.
    if (preview) {
      const { diffAdded: added, diffRemoved: removed, diffChanged: changed } = theme;

      const block = (note: Note, color: string, filled: boolean) => {
        const rect = noteRect(note, 0, view);
        if (rect.x + rect.width < KEY_WIDTH || rect.x > width) return;
        if (rect.y + rect.height < RULER_HEIGHT || rect.y > rollHeight) return;

        const x = Math.max(KEY_WIDTH, rect.x);
        const drawn = Math.max(1, rect.width - (x - rect.x));

        if (filled) {
          ctx.globalAlpha = 0.85;
          ctx.fillStyle = color;
          ctx.fillRect(x, rect.y + 1, drawn, rect.height - 2);
          ctx.globalAlpha = 1;
        } else {
          ctx.globalAlpha = 0.9;
          ctx.strokeStyle = color;
          ctx.lineWidth = 1;
          ctx.setLineDash([3, 2]);
          ctx.strokeRect(x + 0.5, rect.y + 1.5, drawn - 1, rect.height - 3);
          ctx.setLineDash([]);
          ctx.globalAlpha = 1;
        }
      };

      preview.removed.forEach((note) => block(note, removed, false));
      preview.changed.forEach(({ before }) => block(before, changed, false));
      preview.changed.forEach(({ after }) => block(after, changed, true));
      preview.added.forEach((note) => block(note, added, true));
    }

    // ---- velocity lane ----
    if (velocityLane > 0) {
      const laneTop = rollHeight;
      ctx.fillStyle = theme.bg1;
      ctx.fillRect(0, laneTop, width, velocityLane);
      ctx.strokeStyle = theme.border;
      ctx.beginPath();
      ctx.moveTo(0, laneTop + 0.5);
      ctx.lineTo(width, laneTop + 0.5);
      ctx.stroke();

      ctx.fillStyle = theme.text2;
      ctx.font = "10px ui-monospace, monospace";
      ctx.fillText("VELOCITY", 6, laneTop + 14);

      const laneBottom = laneTop + velocityLane - 6;
      const laneHeight = velocityLane - 22;

      track.notes.forEach((note, index) => {
        const x = tickToX(note.start_ticks, view);
        if (x < KEY_WIDTH || x > width) return;
        const height = (note.velocity / 127) * laneHeight;
        ctx.fillStyle = selection.has(index) ? theme.accent : track.color;
        ctx.globalAlpha = selection.has(index) ? 1 : 0.7;
        ctx.fillRect(x, laneBottom - height, 3, height);
      });
      ctx.globalAlpha = 1;
    }

    // ---- ruler ----
    ctx.fillStyle = theme.bg1;
    ctx.fillRect(0, 0, width, RULER_HEIGHT);
    ctx.strokeStyle = theme.border;
    ctx.beginPath();
    ctx.moveTo(0, RULER_HEIGHT + 0.5);
    ctx.lineTo(width, RULER_HEIGHT + 0.5);
    ctx.stroke();

    ctx.fillStyle = theme.text2;
    ctx.font = "10px ui-monospace, monospace";
    for (let tick = Math.floor(startTick / barTicks) * barTicks; tick <= endTick; tick += barTicks) {
      const x = tickToX(tick, view);
      if (x < KEY_WIDTH) continue;
      ctx.fillText(String(Math.floor(tick / barTicks) + 1), x + 3, 14);
      ctx.strokeStyle = theme.gridBar;
      ctx.beginPath();
      ctx.moveTo(Math.round(x) + 0.5, 0);
      ctx.lineTo(Math.round(x) + 0.5, RULER_HEIGHT);
      ctx.stroke();
    }

    // ---- keyboard gutter ----
    ctx.fillStyle = theme.bg1;
    ctx.fillRect(0, RULER_HEIGHT, KEY_WIDTH, height - RULER_HEIGHT);

    for (let pitch = firstPitch; pitch >= lastPitch; pitch -= 1) {
      const y = pitchToY(pitch, view);
      if (y > rollHeight || y + rowHeight < RULER_HEIGHT) continue;
      const black = isBlackKey(pitch);
      ctx.fillStyle = black ? theme.keyBlack : theme.keyWhite;
      ctx.fillRect(0, y, black ? KEY_WIDTH * 0.62 : KEY_WIDTH - 1, rowHeight - 1);

      if (pitch % 12 === 0 && rowHeight >= 9) {
        ctx.fillStyle = theme.text2;
        ctx.font = "9px ui-monospace, monospace";
        ctx.fillText(pitchName(pitch), KEY_WIDTH - 22, y + rowHeight - 2);
      }
    }

    ctx.strokeStyle = theme.border;
    ctx.beginPath();
    ctx.moveTo(KEY_WIDTH + 0.5, 0);
    ctx.lineTo(KEY_WIDTH + 0.5, height);
    ctx.stroke();

    // ---- marquee ----
    if (marquee !== null) {
      const { left, top, right, bottom } = normalizeMarquee(marquee);
      ctx.fillStyle = "rgba(91,141,217,0.16)";
      ctx.fillRect(left, top, right - left, bottom - top);
      ctx.strokeStyle = theme.accent;
      ctx.setLineDash([3, 3]);
      ctx.strokeRect(left + 0.5, top + 0.5, right - left, bottom - top);
      ctx.setLineDash([]);
    }

    // ---- loop region ----
    if (loopRegion) {
      const [loopStart, loopEnd] = loopRegion;
      const startX = Math.max(KEY_WIDTH, tickToX(loopStart, view));
      const endX = Math.min(width, tickToX(loopEnd, view));

      if (endX > startX) {
        // A wash over the looped span, so it reads at a glance without obscuring notes.
        ctx.fillStyle = "rgba(217,164,65,0.07)";
        ctx.fillRect(startX, RULER_HEIGHT, endX - startX, rollHeight - RULER_HEIGHT);

        ctx.fillStyle = theme.playhead;
        ctx.fillRect(startX, 0, 2, RULER_HEIGHT);
        ctx.fillRect(endX - 2, 0, 2, RULER_HEIGHT);
        ctx.globalAlpha = 0.35;
        ctx.fillRect(startX, 0, endX - startX, RULER_HEIGHT);
        ctx.globalAlpha = 1;
      }
    }

    // ---- playhead ----
    const playheadX = tickToX(playheadTicks, view);
    if (playheadX >= KEY_WIDTH && playheadX <= width) {
      ctx.strokeStyle = theme.playhead;
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.moveTo(playheadX, 0);
      ctx.lineTo(playheadX, height);
      ctx.stroke();
    }
}
