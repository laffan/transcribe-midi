import Foundation

/// The C ABI that `crates/unplugged-audio/src/backend.rs` declares.
///
/// Every function returns 0 on success and non-zero on failure, with detail available
/// from `unplugged_audio_last_error`. Handles are opaque `Unmanaged` pointers to an
/// `AudioGraph`; Rust never dereferences them.
///
/// These are the *only* entry points. Keeping the surface this small is what lets one
/// Swift implementation serve both macOS and iOS without divergence.

@_cdecl("unplugged_audio_create")
public func unplugged_audio_create() -> UnsafeMutableRawPointer? {
    let graph = AudioGraph()
    return Unmanaged.passRetained(graph).toOpaque()
}

@_cdecl("unplugged_audio_destroy")
public func unplugged_audio_destroy(_ handle: UnsafeMutableRawPointer?) {
    guard let handle else { return }
    Unmanaged<AudioGraph>.fromOpaque(handle).release()
}

@_cdecl("unplugged_audio_start")
public func unplugged_audio_start(
    _ handle: UnsafeMutableRawPointer?,
    _ renderContext: UnsafeMutableRawPointer?
) -> Int32 {
    guard let graph = graph(handle), let renderContext else { return -1 }
    return graph.start(transport: renderContext) ? 0 : 1
}

@_cdecl("unplugged_audio_stop")
public func unplugged_audio_stop(_ handle: UnsafeMutableRawPointer?) -> Int32 {
    guard let graph = graph(handle) else { return -1 }
    graph.stop()
    return 0
}

@_cdecl("unplugged_audio_sample_rate")
public func unplugged_audio_sample_rate(_ handle: UnsafeMutableRawPointer?) -> Double {
    guard let graph = graph(handle) else { return 0 }
    return graph.sampleRate
}

@_cdecl("unplugged_audio_ensure_tracks")
public func unplugged_audio_ensure_tracks(_ handle: UnsafeMutableRawPointer?, _ count: UInt32) -> Int32 {
    guard let graph = graph(handle) else { return -1 }
    return graph.ensureTracks(Int(count)) ? 0 : 1
}

@_cdecl("unplugged_audio_note_on")
public func unplugged_audio_note_on(
    _ handle: UnsafeMutableRawPointer?,
    _ track: UInt16,
    _ pitch: UInt8,
    _ velocity: UInt8,
    _ channel: UInt8
) -> Int32 {
    guard let graph = graph(handle) else { return -1 }
    return graph.noteOn(track: Int(track), pitch: pitch, velocity: velocity, channel: channel) ? 0 : 1
}

@_cdecl("unplugged_audio_note_off")
public func unplugged_audio_note_off(
    _ handle: UnsafeMutableRawPointer?,
    _ track: UInt16,
    _ pitch: UInt8,
    _ channel: UInt8
) -> Int32 {
    guard let graph = graph(handle) else { return -1 }
    return graph.noteOff(track: Int(track), pitch: pitch, channel: channel) ? 0 : 1
}

@_cdecl("unplugged_audio_all_notes_off")
public func unplugged_audio_all_notes_off(_ handle: UnsafeMutableRawPointer?) -> Int32 {
    guard let graph = graph(handle) else { return -1 }
    graph.allNotesOff()
    return 0
}

@_cdecl("unplugged_audio_set_track_gain")
public func unplugged_audio_set_track_gain(
    _ handle: UnsafeMutableRawPointer?,
    _ track: UInt16,
    _ gain: Float
) -> Int32 {
    guard let graph = graph(handle) else { return -1 }
    return graph.setTrackGain(track: Int(track), gain: gain) ? 0 : 1
}

/// Copy the most recent error into `buf`, returning the byte count written.
/// Truncates rather than overrunning; the message is UTF-8 and not NUL-terminated.
@_cdecl("unplugged_audio_last_error")
public func unplugged_audio_last_error(
    _ handle: UnsafeMutableRawPointer?,
    _ buf: UnsafeMutablePointer<UInt8>?,
    _ capacity: UInt32
) -> UInt32 {
    guard let graph = graph(handle), let buf, capacity > 0 else { return 0 }

    let bytes = Array(graph.takeLastError().utf8)
    let count = min(bytes.count, Int(capacity))
    guard count > 0 else { return 0 }

    bytes.withUnsafeBufferPointer { source in
        guard let base = source.baseAddress else { return }
        buf.update(from: base, count: count)
    }
    return UInt32(count)
}

private func graph(_ handle: UnsafeMutableRawPointer?) -> AudioGraph? {
    guard let handle else { return nil }
    return Unmanaged<AudioGraph>.fromOpaque(handle).takeUnretainedValue()
}
