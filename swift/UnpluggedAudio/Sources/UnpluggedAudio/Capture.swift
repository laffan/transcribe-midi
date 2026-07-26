import AVFoundation
import Foundation

#if os(macOS)
import AppKit
#endif

/// Phase 7: microphone capture for transcription.
///
/// Deliberately a *separate* `AVAudioEngine` from the playback graph. Sharing one would
/// mean reconfiguring the running engine to attach an input tap, which on iOS forces the
/// audio session into `.playAndRecord` for the life of the app — a permission prompt and
/// a routing change (speaker to receiver, on some devices) that a user who never
/// transcribes anything should not be subjected to. Two engines cost one extra render
/// thread while recording and nothing at all when not.
///
/// The spec is explicit that recorded audio is not a feature: nothing here writes a file
/// or keeps a take. Samples are captured, drained into Rust, analysed, and dropped.
public final class AudioCapture {
    /// Ceiling on retained audio, in seconds.
    ///
    /// Rust drains continuously while recording, so this only bounds what a stall could
    /// accumulate. Two minutes at 48 kHz is about 23 MB — generous for a phrase, and
    /// small enough that a forgotten recording cannot exhaust memory on a phone.
    private static let maxSeconds = 120.0

    private let engine = AVAudioEngine()
    private let lock = NSLock()

    /// Captured mono samples not yet drained.
    private var pending: [Float] = []
    private var capturing = false
    private var captureSampleRate: Double = 0
    /// Peak level since the last read, for the level meter.
    private var peak: Float = 0

    private var lastError = ""

    public init() {}

    deinit {
        stop()
    }

    public func errorMessage() -> String {
        lock.lock()
        defer { lock.unlock() }
        return lastError
    }

    private func fail(_ message: String) -> Int32 {
        lock.lock()
        lastError = message
        lock.unlock()
        return 1
    }

    // -- lifecycle ----------------------------------------------------------

    /// Begin capturing. Idempotent.
    ///
    /// Returns 0 on success, 2 when microphone access has not been granted, and 1 for
    /// anything else with a message available from `errorMessage()`. Permission is
    /// distinguished because it is the one failure the user can do something about, and
    /// the UI says so rather than showing a generic error.
    public func start() -> Int32 {
        lock.lock()
        let alreadyRunning = capturing
        lock.unlock()
        if alreadyRunning {
            return 0
        }

        guard hasMicrophonePermission() else {
            return 2
        }

        #if os(iOS)
        do {
            let session = AVAudioSession.sharedInstance()
            // `.playAndRecord` rather than `.record` so the metronome and the built-in
            // sampler still sound while transcribing; `.measurement` disables the voice
            // processing that would otherwise gate and EQ the signal, which is exactly
            // the processing that would ruin a pitch estimate.
            try session.setCategory(.playAndRecord, mode: .measurement, options: [.defaultToSpeaker])
            try session.setActive(true)
        } catch {
            return fail("could not configure the audio session: \(error.localizedDescription)")
        }
        #endif

        let input = engine.inputNode
        let format = input.outputFormat(forBus: 0)
        guard format.sampleRate > 0, format.channelCount > 0 else {
            return fail("no microphone input is available")
        }

        lock.lock()
        pending.removeAll(keepingCapacity: true)
        captureSampleRate = format.sampleRate
        peak = 0
        lock.unlock()

        let limit = Int(format.sampleRate * Self.maxSeconds)

        input.installTap(onBus: 0, bufferSize: 4096, format: format) { [weak self] buffer, _ in
            guard let self, let channels = buffer.floatChannelData else { return }

            let frames = Int(buffer.frameLength)
            let channelCount = Int(buffer.format.channelCount)
            if frames == 0 { return }

            // Downmix to mono. Transcription is monophonic and a stereo pair of the same
            // source would only halve the work done per sample.
            var mono = [Float](repeating: 0, count: frames)
            for channel in 0..<channelCount {
                let samples = channels[channel]
                for frame in 0..<frames {
                    mono[frame] += samples[frame]
                }
            }
            if channelCount > 1 {
                let scale = 1.0 / Float(channelCount)
                for frame in 0..<frames {
                    mono[frame] *= scale
                }
            }

            var localPeak: Float = 0
            for sample in mono {
                localPeak = max(localPeak, abs(sample))
            }

            self.lock.lock()
            if self.pending.count + frames <= limit {
                self.pending.append(contentsOf: mono)
            }
            self.peak = max(self.peak, localPeak)
            self.lock.unlock()
        }

        engine.prepare()
        do {
            try engine.start()
        } catch {
            input.removeTap(onBus: 0)
            return fail("could not start the microphone: \(error.localizedDescription)")
        }

        lock.lock()
        capturing = true
        lock.unlock()
        return 0
    }

    /// Stop capturing and discard anything not yet drained. Idempotent.
    public func stop() {
        lock.lock()
        let wasCapturing = capturing
        capturing = false
        lock.unlock()

        guard wasCapturing else { return }

        engine.inputNode.removeTap(onBus: 0)
        engine.stop()

        #if os(iOS)
        // Hand the session back, or playback stays in the record category and the route
        // never returns to normal.
        try? AVAudioSession.sharedInstance().setActive(
            false,
            options: .notifyOthersOnDeactivation
        )
        #endif
    }

    // -- draining -----------------------------------------------------------

    /// Copy up to `capacity` samples out and remove them from the queue.
    public func drain(into destination: UnsafeMutablePointer<Float>, capacity: Int) -> Int {
        lock.lock()
        defer { lock.unlock() }

        let count = min(capacity, pending.count)
        if count == 0 { return 0 }

        pending.withUnsafeBufferPointer { source in
            destination.update(from: source.baseAddress!, count: count)
        }
        pending.removeFirst(count)
        return count
    }

    public func sampleRate() -> Double {
        lock.lock()
        defer { lock.unlock() }
        return captureSampleRate
    }

    /// Peak level since the last call, and reset. Drives the level meter.
    public func takePeak() -> Float {
        lock.lock()
        defer { lock.unlock() }
        let value = peak
        peak = 0
        return value
    }

    // -- permission ---------------------------------------------------------

    /// Whether the microphone may be used, asking for it the first time.
    ///
    /// Blocking, because the Rust caller is on a worker thread and the alternative is a
    /// callback across the C ABI for a prompt the user answers once. It is never called
    /// from the audio thread or the main thread.
    private func hasMicrophonePermission() -> Bool {
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized:
            return true
        case .denied, .restricted:
            return false
        case .notDetermined:
            let semaphore = DispatchSemaphore(value: 0)
            var granted = false
            AVCaptureDevice.requestAccess(for: .audio) { allowed in
                granted = allowed
                semaphore.signal()
            }
            semaphore.wait()
            return granted
        @unknown default:
            return false
        }
    }
}

// ---------------------------------------------------------------------------
// C ABI
// ---------------------------------------------------------------------------
//
// Same shape as the rest of the audio surface: an opaque handle created and destroyed by
// Rust, and functions that take it back. No global state, so a second capture in a test
// harness would not fight the first.

@_cdecl("unplugged_capture_create")
public func unplugged_capture_create() -> UnsafeMutableRawPointer {
    Unmanaged.passRetained(AudioCapture()).toOpaque()
}

@_cdecl("unplugged_capture_destroy")
public func unplugged_capture_destroy(_ handle: UnsafeMutableRawPointer?) {
    guard let handle else { return }
    Unmanaged<AudioCapture>.fromOpaque(handle).takeRetainedValue().stop()
}

@_cdecl("unplugged_capture_start")
public func unplugged_capture_start(_ handle: UnsafeMutableRawPointer?) -> Int32 {
    guard let handle else { return -1 }
    return Unmanaged<AudioCapture>.fromOpaque(handle).takeUnretainedValue().start()
}

@_cdecl("unplugged_capture_stop")
public func unplugged_capture_stop(_ handle: UnsafeMutableRawPointer?) -> Int32 {
    guard let handle else { return -1 }
    Unmanaged<AudioCapture>.fromOpaque(handle).takeUnretainedValue().stop()
    return 0
}

@_cdecl("unplugged_capture_drain")
public func unplugged_capture_drain(
    _ handle: UnsafeMutableRawPointer?,
    _ destination: UnsafeMutablePointer<Float>?,
    _ capacity: UInt32
) -> UInt32 {
    guard let handle, let destination else { return 0 }
    let capture = Unmanaged<AudioCapture>.fromOpaque(handle).takeUnretainedValue()
    return UInt32(capture.drain(into: destination, capacity: Int(capacity)))
}

@_cdecl("unplugged_capture_sample_rate")
public func unplugged_capture_sample_rate(_ handle: UnsafeMutableRawPointer?) -> Double {
    guard let handle else { return 0 }
    return Unmanaged<AudioCapture>.fromOpaque(handle).takeUnretainedValue().sampleRate()
}

@_cdecl("unplugged_capture_take_peak")
public func unplugged_capture_take_peak(_ handle: UnsafeMutableRawPointer?) -> Float {
    guard let handle else { return 0 }
    return Unmanaged<AudioCapture>.fromOpaque(handle).takeUnretainedValue().takePeak()
}

/// Last error, as a malloc'd C string, or NULL. Free with
/// `unplugged_platform_string_free`.
@_cdecl("unplugged_capture_last_error")
public func unplugged_capture_last_error(
    _ handle: UnsafeMutableRawPointer?
) -> UnsafeMutablePointer<CChar>? {
    guard let handle else { return nil }
    let message = Unmanaged<AudioCapture>.fromOpaque(handle).takeUnretainedValue().errorMessage()
    return message.isEmpty ? nil : strdup(message)
}
