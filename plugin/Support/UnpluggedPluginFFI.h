// The Rust symbols the AUv3 extension calls.
//
// Reached from Swift through the bridging header rather than @_silgen_name, so both sides
// of the boundary are type-checked by the compiler. Implemented in
// crates/unplugged-plugin/src/lib.rs and linked as a static archive.

#ifndef UNPLUGGED_PLUGIN_FFI_H
#define UNPLUGGED_PLUGIN_FFI_H

#include <stdbool.h>
#include <stdint.h>

/// Mirrors `CRenderedEvent` in crates/unplugged-plugin/src/lib.rs.
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

/// Create a plugin instance rooted at a projects directory. NULL on failure.
void *unplugged_plugin_create(const char *data_dir);
void unplugged_plugin_destroy(void *handle);

/// Free any string returned by the functions below.
void unplugged_plugin_string_free(char *pointer);

/// JSON array of available projects. Caller frees.
char *unplugged_plugin_projects_json(void *handle);

/// Open a project. 0 on success; otherwise see `unplugged_plugin_last_error`.
int32_t unplugged_plugin_open(void *handle, const char *id);

/// Last error, or NULL. Caller frees. Reading clears it.
char *unplugged_plugin_last_error(void *handle);

/// Sample rate changed, or render resources were reallocated.
void unplugged_plugin_prepare(void *handle, double sample_rate);

uint32_t unplugged_plugin_track_count(void *handle);

/// The host's `fullState`, as JSON. Caller frees.
char *unplugged_plugin_state_json(void *handle);
int32_t unplugged_plugin_set_state_json(void *handle, const char *json);

/// Version, commit and dirty flag of the running code. Caller frees.
char *unplugged_plugin_build_info_json(void);

/// One render block.
///
/// REALTIME: called from the audio thread. Allocation-free and lock-free on the Rust
/// side, and it never panics across the boundary. Returns the number of events written.
uint32_t unplugged_plugin_render(void *handle,
                                 double host_beats,
                                 double tempo_bpm,
                                 bool playing,
                                 uint32_t frames,
                                 UnpluggedRenderedEvent *out,
                                 uint32_t capacity);

#endif /* UNPLUGGED_PLUGIN_FFI_H */
