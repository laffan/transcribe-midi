// SwiftPM requires at least one source file in a C target; the content lives entirely
// in include/unplugged_ffi.h. The symbols themselves come from the Rust staticlib.
//
// The include directory is on this target's header search path, so the header is
// referenced by name rather than by relative path.
#include "unplugged_ffi.h"
