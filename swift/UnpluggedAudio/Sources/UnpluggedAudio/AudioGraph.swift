import AVFoundation
import CUnpluggedFFI
import Foundation

#if os(iOS)
import UIKit
#endif

/// The AVAudioEngine graph and its render callback.
///
/// Ownership split, per the spec: Rust owns the sequencer and decides *when* notes fire;
/// this class owns the graph and the render callback and decides *how* they sound.
///
/// Graph shape (one chain per track, so Phase 9 can swap an AUv3 in where the sampler
/// sits without touching anything downstream):
///
///     [sampler] -> [track mixer (gain)] -> [main mixer] -> [output]
///
/// AVAudioEngine is used rather than `cpal` because hosted Audio Units expect to live in
/// an AVAudioEngine graph, and retrofitting that later would mean rewriting the engine.
public final class AudioGraph {
    /// Ceiling on events pulled from Rust per render quantum. Preallocated — the audio
    /// thread must not allocate.
    private static let maxEventsPerBuffer = 512

    private let engine = AVAudioEngine()
    private var tracks: [TrackChain] = []

    /// Serialises graph mutation. The audio thread never touches it.
    private let graphQueue = DispatchQueue(label: "com.unplugged.audio.graph")

    /// Opaque pointer to the Rust `SharedTransport`. Read on the audio thread.
    private var transport: UnsafeMutableRawPointer?
    /// Opaque pointer to the Rust `RenderState`. Owned by the audio thread.
    private var renderState: UnsafeMutableRawPointer?

    private var eventBuffer: UnsafeMutablePointer<UnpluggedRenderedEvent>
    private var renderNotifyInstalled = false
    private var isRunning = false

    private var lastError: String = ""
    private let errorLock = NSLock()

    private struct TrackChain {
        let sampler: AVAudioUnitSampler
        let mixer: AVAudioMixerNode
    }

    public init() {
        eventBuffer = UnsafeMutablePointer<UnpluggedRenderedEvent>.allocate(
            capacity: Self.maxEventsPerBuffer
        )
        eventBuffer.initialize(
            repeating: UnpluggedRenderedEvent(
                frame_offset: 0, track: 0, kind: 0, pitch: 0, velocity: 0, channel: 0, _pad: (0, 0)
            ),
            count: Self.maxEventsPerBuffer
        )
    }

    deinit {
        stop()
        eventBuffer.deinitialize(count: Self.maxEventsPerBuffer)
        eventBuffer.deallocate()
        if let renderState {
            unplugged_audio_render_state_destroy(renderState)
        }
    }

    // MARK: - Errors

    private func setError(_ message: String) {
        errorLock.lock()
        lastError = message
        errorLock.unlock()
    }

    public func takeLastError() -> String {
        errorLock.lock()
        defer {
            lastError = ""
            errorLock.unlock()
        }
        return lastError
    }

    // MARK: - Lifecycle

    public var sampleRate: Double {
        engine.outputNode.outputFormat(forBus: 0).sampleRate
    }

    /// Start the engine and install the render callback.
    ///
    /// `transport` is the Rust `SharedTransport` pointer; it must outlive this graph.
    public func start(transport: UnsafeMutableRawPointer) -> Bool {
        var ok = false
        graphQueue.sync {
            guard !isRunning else {
                ok = true
                return
            }
            self.transport = transport

            #if os(iOS)
            // Without an active session in `.playback` there is no audible output at
            // all on iOS, and the app will not keep running in the background.
            // Requires UIBackgroundModes = ["audio"] in Info.plist.
            do {
                let session = AVAudioSession.sharedInstance()
                try session.setCategory(.playAndRecord,
                                        mode: .default,
                                        options: [.mixWithOthers, .defaultToSpeaker, .allowBluetoothA2DP])
                try session.setActive(true)
            } catch {
                setError("could not activate the audio session: \(error.localizedDescription)")
                return
            }
            #endif

            // At least one track must exist before starting, or the graph has no input
            // to the main mixer and CoreAudio may refuse to start.
            if tracks.isEmpty {
                addTrackChain()
            }

            if renderState == nil {
                renderState = unplugged_audio_render_state_create(sampleRate, 480, 120.0)
            }

            installRenderNotifyIfNeeded()

            engine.prepare()
            do {
                try engine.start()
                isRunning = true
                ok = true
            } catch {
                setError("could not start the audio engine: \(error.localizedDescription)")
            }
        }
        return ok
    }

    public func stop() {
        graphQueue.sync {
            guard isRunning else { return }
            allNotesOff()
            engine.stop()
            isRunning = false
            #if os(iOS)
            try? AVAudioSession.sharedInstance().setActive(false)
            #endif
        }
    }

    public var running: Bool {
        graphQueue.sync { isRunning }
    }

    // MARK: - Graph construction

    /// Ensure at least `count` track chains exist.
    public func ensureTracks(_ count: Int) -> Bool {
        var ok = true
        graphQueue.sync {
            while tracks.count < count {
                if !addTrackChain() {
                    ok = false
                    return
                }
            }
        }
        return ok
    }

    @discardableResult
    private func addTrackChain() -> Bool {
        let sampler = AVAudioUnitSampler()
        let mixer = AVAudioMixerNode()

        engine.attach(sampler)
        engine.attach(mixer)

        let format = engine.outputNode.outputFormat(forBus: 0)
        engine.connect(sampler, to: mixer, format: format)
        engine.connect(mixer, to: engine.mainMixerNode, format: format)

        loadDefaultInstrument(into: sampler)
        tracks.append(TrackChain(sampler: sampler, mixer: mixer))
        return true
    }

    /// Load the bundled instrument.
    ///
    /// The spec calls this the *test* instrument, not a feature, so the failure path is
    /// deliberately soft: with no SF2 bundled, `AVAudioUnitSampler` still produces its
    /// built-in default tone. That keeps the app audible before the sample set is added
    /// and keeps the instrument swappable, which is the actual requirement.
    private func loadDefaultInstrument(into sampler: AVAudioUnitSampler) {
        guard let url = Bundle.main.url(forResource: "Piano", withExtension: "sf2")
            ?? Bundle.main.url(forResource: "GeneralUser", withExtension: "sf2")
        else {
            setError("no bundled SF2 found; using the sampler's default tone")
            return
        }

        do {
            try sampler.loadSoundBankInstrument(
                at: url,
                program: 0,
                bankMSB: UInt8(kAUSampler_DefaultMelodicBankMSB),
                bankLSB: UInt8(kAUSampler_DefaultBankLSB)
            )
        } catch {
            setError("could not load \(url.lastPathComponent): \(error.localizedDescription)")
        }
    }

    public func setTrackGain(track: Int, gain: Float) -> Bool {
        var ok = false
        graphQueue.sync {
            guard track >= 0, track < tracks.count else { return }
            tracks[track].mixer.outputVolume = max(0, min(gain, 1))
            ok = true
        }
        return ok
    }

    // MARK: - Live input
    //
    // The on-screen keyboard and (from Phase 4) external MIDI input bypass the
    // sequencer entirely — they are not on the timeline, so they play immediately.

    public func noteOn(track: Int, pitch: UInt8, velocity: UInt8, channel: UInt8) -> Bool {
        var ok = false
        graphQueue.sync {
            guard let sampler = sampler(at: track) else { return }
            sampler.startNote(pitch, withVelocity: velocity, onChannel: channel)
            ok = true
        }
        return ok
    }

    public func noteOff(track: Int, pitch: UInt8, channel: UInt8) -> Bool {
        var ok = false
        graphQueue.sync {
            guard let sampler = sampler(at: track) else { return }
            sampler.stopNote(pitch, onChannel: channel)
            ok = true
        }
        return ok
    }

    public func allNotesOff() {
        for chain in tracks {
            for channel in UInt8(0)...UInt8(15) {
                // CC 123 = All Notes Off. Sent on every channel because a track's notes
                // may have been recorded on any of them.
                chain.sampler.sendController(123, withValue: 0, onChannel: channel)
            }
        }
    }

    private func sampler(at index: Int) -> AVAudioUnitSampler? {
        guard index >= 0, index < tracks.count else { return nil }
        return tracks[index].sampler
    }

    // MARK: - Render callback

    /// Install a pre-render notify on the main mixer.
    ///
    /// This fires on the audio thread once per render quantum, *before* the mixer pulls
    /// its inputs — so MIDI scheduled here lands in the buffer being rendered right now.
    /// That is what makes output sample-accurate rather than buffer-quantised.
    private func installRenderNotifyIfNeeded() {
        guard !renderNotifyInstalled else { return }

        let unit = engine.mainMixerNode.audioUnit
        guard let unit else {
            setError("main mixer has no audio unit; cannot install the render callback")
            return
        }

        let context = Unmanaged.passUnretained(self).toOpaque()

        let status = AudioUnitAddRenderNotify(
            unit,
            { (inRefCon, ioActionFlags, _, _, inNumberFrames, _) -> OSStatus in
                // ---- AUDIO THREAD ----
                // No allocation, no locks, no Swift runtime calls that could allocate.
                guard ioActionFlags.pointee.contains(.unitRenderAction_PreRender) else {
                    return noErr
                }
                let graph = Unmanaged<AudioGraph>.fromOpaque(inRefCon).takeUnretainedValue()
                graph.renderTick(frames: inNumberFrames)
                return noErr
            },
            context
        )

        if status != noErr {
            setError("AudioUnitAddRenderNotify failed with status \(status)")
            return
        }
        renderNotifyInstalled = true
    }

    /// Pull this buffer's events from Rust and schedule them.
    ///
    /// REALTIME — audio thread. Everything it touches is preallocated.
    private func renderTick(frames: AVAudioFrameCount) {
        guard let transport, let renderState else { return }

        let count = unplugged_audio_render(
            transport,
            renderState,
            UInt32(frames),
            eventBuffer,
            UInt32(Self.maxEventsPerBuffer)
        )
        guard count > 0 else { return }

        for index in 0..<Int(count) {
            let event = eventBuffer[index]
            let trackIndex = Int(event.track)
            guard trackIndex < tracks.count else { continue }

            guard let schedule = tracks[trackIndex].sampler.auAudioUnit.scheduleMIDIEventBlock else {
                continue
            }

            // 0x90 = note on, 0x80 = note off, OR'd with the channel.
            let status: UInt8 = (event.kind == 1 ? 0x90 : 0x80) | (event.channel & 0x0F)
            var bytes: (UInt8, UInt8, UInt8) = (status, event.pitch & 0x7F, event.velocity & 0x7F)

            withUnsafeBytes(of: &bytes) { raw in
                guard let base = raw.baseAddress?.assumingMemoryBound(to: UInt8.self) else { return }
                // `AUEventSampleTimeImmediate + offset` is the documented way to place
                // an event at a sample offset within the buffer currently rendering.
                schedule(AUEventSampleTimeImmediate + Int64(event.frame_offset), 0, 3, base)
            }
        }
    }
}
