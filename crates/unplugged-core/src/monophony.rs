//! Making a list of notes playable by one voice.
//!
//! A transcription is monophonic by construction — the segmenter walks a single frame
//! track, so its notes cannot overlap. Two things break that afterwards. Quantisation
//! rounds a start backwards and a length up, and two sixteenths a grid step apart can
//! land on top of each other. Dragging in the fine-tune editor can put a note anywhere
//! at all.
//!
//! Either way the result is a lie about what was performed: the source was one voice, and
//! two notes sounding at once is something that could not have been sung. So the rule is
//! enforced rather than hoped for, in one place, over a plain note list.

use crate::{Note, Ticks};

/// Sort key: earliest first, and where two notes start together the longer one leads.
///
/// The tie-break is what decides which note survives a same-start collision, because the
/// leader is the one that gets truncated to nothing and dropped. Keeping the longer note
/// is the better guess about what was played: a stray short note on the same attack is
/// far more often a detection artefact than a real event.
fn order_key(note: &Note) -> (u32, std::cmp::Reverse<u32>, u8) {
    (
        note.start_ticks,
        std::cmp::Reverse(note.duration_ticks),
        note.pitch,
    )
}

/// Trim and drop notes until no two sound at once.
///
/// A note that runs into the next one's attack is cut short there — the later attack
/// wins, which is what a monophonic instrument does. A note left with nothing at all is
/// dropped rather than kept at zero length, which is unrepresentable in SMF anyway.
pub fn flatten(notes: &[Note]) -> Vec<Note> {
    flatten_indexed(notes)
        .into_iter()
        .map(|(index, duration_ticks)| Note {
            duration_ticks,
            ..notes[index]
        })
        .collect()
}

/// [`flatten`], reported as decisions rather than as notes: which of the originals
/// survive, in order, and how long each one now lasts.
///
/// For callers carrying more than a note — a transcription's detected notes have the
/// analysis behind them attached, and rebuilding those by matching notes back up would
/// be a second, worse version of this function.
pub fn flatten_indexed(notes: &[Note]) -> Vec<(usize, Ticks)> {
    let mut ordered: Vec<usize> = (0..notes.len()).collect();
    ordered.sort_by_key(|&index| order_key(&notes[index]));

    let mut kept: Vec<(usize, Ticks)> = Vec::with_capacity(ordered.len());
    for index in ordered {
        let note = &notes[index];
        if let Some(&(previous, duration)) = kept.last() {
            let start = notes[previous].start_ticks;
            if note.start_ticks < start + duration {
                // Sorted ascending, so this can only be an exact tie — and the sort has
                // already put the longer of the two in `kept`. Dropping the newcomer
                // rather than truncating it to nothing is the same decision, said once.
                if note.start_ticks == start {
                    continue;
                }
                let last = kept.len() - 1;
                kept[last].1 = note.start_ticks - start;
            }
        }
        kept.push((index, note.duration_ticks));
    }

    kept
}

/// Whether a list already sounds one note at a time. The postcondition of [`flatten`].
pub fn is_monophonic(notes: &[Note]) -> bool {
    let mut ordered: Vec<&Note> = notes.iter().collect();
    ordered.sort_by_key(|note| order_key(note));
    ordered
        .windows(2)
        .all(|pair| pair[0].start_ticks + pair[0].duration_ticks <= pair[1].start_ticks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(pitch: u8, start: u32, duration: u32) -> Note {
        Note::new(pitch, start, duration, 90, 0).unwrap()
    }

    #[test]
    fn a_list_that_does_not_overlap_is_returned_unchanged() {
        let notes = vec![note(60, 0, 100), note(62, 100, 100), note(64, 400, 50)];
        assert_eq!(flatten(&notes), notes);
        assert!(is_monophonic(&notes));
    }

    #[test]
    fn a_note_running_into_the_next_attack_is_cut_there() {
        let flat = flatten(&[note(60, 0, 480), note(64, 200, 480)]);
        assert_eq!(flat.len(), 2);
        assert_eq!(flat[0].duration_ticks, 200, "the later attack wins");
        assert_eq!(flat[1].start_ticks, 200);
        assert!(is_monophonic(&flat));
    }

    #[test]
    fn two_notes_at_the_same_moment_leave_the_longer_one() {
        let flat = flatten(&[note(60, 0, 120), note(67, 0, 480)]);
        assert_eq!(flat, vec![note(67, 0, 480)]);
    }

    #[test]
    fn a_note_swallowed_by_a_longer_one_still_interrupts_it() {
        // The long note is cut at the short one's attack and does not resume: a voice
        // that stopped to play something else has stopped.
        let flat = flatten(&[note(60, 0, 960), note(72, 240, 120)]);
        assert_eq!(flat, vec![note(60, 0, 240), note(72, 240, 120)]);
        assert!(is_monophonic(&flat));
    }

    #[test]
    fn a_pile_of_notes_on_one_attack_leaves_exactly_one() {
        let flat = flatten(&[note(60, 96, 480), note(64, 96, 480), note(67, 96, 240)]);
        assert_eq!(flat.len(), 1);
        assert_eq!(flat[0].duration_ticks, 480);
        assert!(is_monophonic(&flat));
    }

    #[test]
    fn quantisation_pileups_flatten_without_losing_the_line() {
        // What snapping to 1/16 does to a run played slightly ahead of the grid: every
        // start rounds to the same place as its neighbour's end, or past it.
        let notes = vec![
            note(60, 0, 120),
            note(62, 110, 120),
            note(64, 230, 120),
            note(65, 330, 120),
        ];
        let flat = flatten(&notes);
        assert_eq!(flat.len(), 4, "no note is lost — they are only shortened");
        assert!(is_monophonic(&flat));
        assert_eq!(flat[0].duration_ticks, 110);
        assert_eq!(flat[3].duration_ticks, 120, "the last one keeps its length");
    }

    #[test]
    fn an_empty_list_is_monophonic() {
        assert!(flatten(&[]).is_empty());
        assert!(is_monophonic(&[]));
    }

    #[test]
    fn the_indexed_form_points_back_at_the_notes_it_kept() {
        // The caller carrying extra data per note must be able to find it again.
        let notes = vec![note(60, 0, 480), note(72, 240, 120), note(64, 240, 60)];
        let kept = flatten_indexed(&notes);
        assert_eq!(kept, vec![(0, 240), (1, 120)]);
        assert_eq!(
            flatten(&notes),
            kept.iter()
                .map(|&(index, duration)| Note {
                    duration_ticks: duration,
                    ..notes[index]
                })
                .collect::<Vec<_>>(),
            "both forms must agree, or the two callers see different rules"
        );
    }

    #[test]
    fn the_result_is_always_sorted_however_the_input_arrived() {
        let flat = flatten(&[note(64, 900, 100), note(60, 0, 100), note(62, 400, 100)]);
        assert_eq!(
            flat.iter().map(|n| n.start_ticks).collect::<Vec<_>>(),
            vec![0, 400, 900]
        );
    }
}
