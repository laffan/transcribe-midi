import AVFoundation
import Foundation

/// Playing a captured take back, so you can hear what you are about to transcribe.
///
/// Its own `AVAudioEngine`, for the same reason capture has one: the playback graph is
/// built around the sequencer and reconfiguring it to schedule a one-off buffer would
/// mean touching a running engine for something that has nothing to do with the project.
/// A player node and an output is the whole graph.
///
/// This is monitoring, not a feature: the take exists to be transcribed and is dropped
/// once its notes are committed. Nothing here writes a file or puts audio on the timeline.
public final class AudioPreview {
    private let engine = AVAudioEngine()
    private let player = AVAudioPlayerNode()
    private let lock = NSLock()

    private var buffer: AVAudioPCMBuffer?
    private var sampleRate: Double = 0
    /// Where in the take the current scheduled buffer started, in samples.
    private var offsetSamples: AVAudioFramePosition = 0
    private var attached = false

    public init() {}

    deinit {
        stop()
        engine.stop()
    }

    /// Hand over a take. Replaces whatever was loaded.
    public func load(samples: UnsafePointer<Float>, count: Int, sampleRate: Double) -> Int32 {
        guard count > 0, sampleRate > 0 else { return -1 }
        guard
            let format = AVAudioFormat(
                commonFormat: .pcmFormatFloat32,
                sampleRate: sampleRate,
                channels: 1,
                interleaved: false
            ),
            let pcm = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(count))
        else {
            return 1
        }

        pcm.frameLength = AVAudioFrameCount(count)
        pcm.floatChannelData![0].update(from: samples, count: count)

        stop()
        lock.lock()
        buffer = pcm
        self.sampleRate = sampleRate
        offsetSamples = 0
        lock.unlock()
        return 0
    }

    /// Play from `fromSeconds`. Idempotent in the sense that it always restarts.
    public func play(fromSeconds: Double) -> Int32 {
        lock.lock()
        let source = buffer
        let rate = sampleRate
        lock.unlock()

        guard let source, rate > 0 else { return 1 }

        stop()

        let start = max(0, min(Int(fromSeconds * rate), Int(source.frameLength) - 1))
        let remaining = Int(source.frameLength) - start
        guard remaining > 0 else { return 0 }

        // A slice rather than seeking: `AVAudioPlayerNode` has no seek, and scheduling the
        // tail of the buffer is how you start partway through.
        guard
            let slice = AVAudioPCMBuffer(pcmFormat: source.format, frameCapacity: AVAudioFrameCount(remaining))
        else {
            return 1
        }
        slice.frameLength = AVAudioFrameCount(remaining)
        slice.floatChannelData![0].update(from: source.floatChannelData![0] + start, count: remaining)

        if !attached {
            engine.attach(player)
            engine.connect(player, to: engine.mainMixerNode, format: source.format)
            attached = true
        }

        #if os(iOS)
        // Capture may have left the session in `.playAndRecord`; either way playback needs
        // it active, and asking is cheap.
        try? AVAudioSession.sharedInstance().setActive(true)
        #endif

        do {
            if !engine.isRunning {
                engine.prepare()
                try engine.start()
            }
        } catch {
            return 2
        }

        lock.lock()
        offsetSamples = AVAudioFramePosition(start)
        lock.unlock()

        player.scheduleBuffer(slice, at: nil, options: [])
        player.play()
        return 0
    }

    public func stop() {
        if player.isPlaying {
            player.stop()
        }
    }

    public func isPlaying() -> Bool {
        player.isPlaying
    }

    /// Current position in the take, in seconds, or -1 when not playing.
    ///
    /// Derived from the player's own render time rather than a wall clock, so the drawn
    /// playhead tracks the audio rather than drifting against it.
    public func positionSeconds() -> Double {
        guard player.isPlaying,
              let nodeTime = player.lastRenderTime,
              let playerTime = player.playerTime(forNodeTime: nodeTime)
        else {
            return -1
        }

        lock.lock()
        let offset = offsetSamples
        let rate = sampleRate
        lock.unlock()

        guard rate > 0 else { return -1 }
        return Double(offset + playerTime.sampleTime) / rate
    }
}

// ---------------------------------------------------------------------------
// C ABI
// ---------------------------------------------------------------------------

@_cdecl("unplugged_preview_create")
public func unplugged_preview_create() -> UnsafeMutableRawPointer {
    Unmanaged.passRetained(AudioPreview()).toOpaque()
}

@_cdecl("unplugged_preview_destroy")
public func unplugged_preview_destroy(_ handle: UnsafeMutableRawPointer?) {
    guard let handle else { return }
    Unmanaged<AudioPreview>.fromOpaque(handle).takeRetainedValue().stop()
}

@_cdecl("unplugged_preview_load")
public func unplugged_preview_load(
    _ handle: UnsafeMutableRawPointer?,
    _ samples: UnsafePointer<Float>?,
    _ count: UInt32,
    _ sampleRate: Double
) -> Int32 {
    guard let handle, let samples else { return -1 }
    return Unmanaged<AudioPreview>.fromOpaque(handle)
        .takeUnretainedValue()
        .load(samples: samples, count: Int(count), sampleRate: sampleRate)
}

@_cdecl("unplugged_preview_play")
public func unplugged_preview_play(
    _ handle: UnsafeMutableRawPointer?,
    _ fromSeconds: Double
) -> Int32 {
    guard let handle else { return -1 }
    return Unmanaged<AudioPreview>.fromOpaque(handle).takeUnretainedValue().play(fromSeconds: fromSeconds)
}

@_cdecl("unplugged_preview_stop")
public func unplugged_preview_stop(_ handle: UnsafeMutableRawPointer?) {
    guard let handle else { return }
    Unmanaged<AudioPreview>.fromOpaque(handle).takeUnretainedValue().stop()
}

/// Position in seconds, or a negative value when not playing.
@_cdecl("unplugged_preview_position")
public func unplugged_preview_position(_ handle: UnsafeMutableRawPointer?) -> Double {
    guard let handle else { return -1 }
    return Unmanaged<AudioPreview>.fromOpaque(handle).takeUnretainedValue().positionSeconds()
}
