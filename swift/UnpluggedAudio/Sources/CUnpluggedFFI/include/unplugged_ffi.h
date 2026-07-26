// Declarations of the Rust symbols the Swift audio graph calls.
//
// These resolve at link time against the Rust staticlib. Kept as a real C header rather
// than @_silgen_name declarations so the types are checked by the compiler on both
// sides of the boundary.

#ifndef UNPLUGGED_FFI_H
#define UNPLUGGED_FFI_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/// Mirrors `CRenderedEvent` in crates/unplugged-audio/src/lib.rs.
/// Field order and padding must match exactly.
typedef struct {
    uint32_t frame_offset;
    uint16_t track;
    uint8_t kind; // 0 = note off, 1 = note on
    uint8_t pitch;
    uint8_t velocity;
    uint8_t channel;
    uint8_t _pad[2];
} UnpluggedRenderedEvent;

/// Create the audio thread's private playback state. Call once, off the audio thread.
void *unplugged_audio_render_state_create(double sample_rate, uint16_t ppq, double tempo_bpm);

/// Release state created above. The audio thread must have stopped first.
void unplugged_audio_render_state_destroy(void *state);

/// Ask the Rust sequencer what happens in the next `frames` samples.
///
/// REALTIME: called from the audio thread every render quantum. The Rust side is
/// allocation-free and lock-free. Returns the number of events written to `out`.
uint32_t unplugged_audio_render(void *transport,
                                void *state,
                                uint32_t frames,
                                UnpluggedRenderedEvent *out,
                                uint32_t capacity);

#ifdef __cplusplus
}
#endif

#endif /* UNPLUGGED_FFI_H */
