import AudioToolbox
import AVFoundation
import CoreAudioKit
import Darwin
import Foundation

/// The Audio Unit itself.
///
/// Type `aumi` — a **MIDI processor**. Unplugged makes notes; it does not make sound in a
/// host, because in a host the sound is whatever instrument you put after it. Logic hosts
/// this in the MIDI FX slot of an instrument track, which is exactly the right place: the
/// notes go into the instrument you already chose.
///
/// The split is the same one the standalone uses and for the same reason. Rust owns the
/// sequencer and decides *what* fires and *when*; this class owns the host relationship —
/// the transport, the render block, the MIDI output — and nothing else. `unplugged_plugin_render`
/// is allocation-free and lock-free, which is what makes it safe to call from here.
public final class UnpluggedAudioUnit: AUAudioUnit {
    /// Events one render block may produce. Preallocated; the audio thread must not
    /// allocate, and neither may we.
    private static let maxEvents = 512

    private var plugin: UnsafeMutableRawPointer?
    private var eventBuffer: UnsafeMutablePointer<UnpluggedRenderedEvent>
    /// The three bytes of the channel-voice message currently being sent. Preallocated for
    /// the same reason as `eventBuffer`: building `[UInt8]` per event would put a heap
    /// allocation in the render path, which is the one thing it must not contain.
    private var messageBuffer: UnsafeMutablePointer<UInt8>

    private var outputBusArray: AUAudioUnitBusArray!
    private var inputBusArray: AUAudioUnitBusArray!

    /// Serialises everything that is not the render block.
    private let stateQueue = DispatchQueue(label: "com.unplugged.au.state")

    public override init(
        componentDescription: AudioComponentDescription,
        options: AudioComponentInstantiationOptions = []
    ) throws {
        eventBuffer = UnsafeMutablePointer<UnpluggedRenderedEvent>.allocate(
            capacity: Self.maxEvents
        )
        eventBuffer.initialize(
            repeating: UnpluggedRenderedEvent(
                frame_offset: 0, track: 0, kind: 0, pitch: 0, velocity: 0, channel: 0, _pad: (0, 0)
            ),
            count: Self.maxEvents
        )
        messageBuffer = UnsafeMutablePointer<UInt8>.allocate(capacity: 3)
        messageBuffer.initialize(repeating: 0, count: 3)

        try super.init(componentDescription: componentDescription, options: options)

        // The projects directory, shared with the standalone app through an App Group.
        // Without the group the extension gets its own sandboxed container and sees no
        // projects at all — which looks exactly like "the app has never run".
        let dataDir = Self.sharedDataDirectory()
        plugin = dataDir.withCString { unplugged_plugin_create($0) }

        // A MIDI processor still needs a bus pair; hosts expect the format negotiation
        // even when no audio flows through.
        let format = AVAudioFormat(standardFormatWithSampleRate: 44100, channels: 2)!
        inputBusArray = AUAudioUnitBusArray(
            audioUnit: self, busType: .input, busses: [try AUAudioUnitBus(format: format)]
        )
        outputBusArray = AUAudioUnitBusArray(
            audioUnit: self, busType: .output, busses: [try AUAudioUnitBus(format: format)]
        )
    }

    deinit {
        if let plugin {
            unplugged_plugin_destroy(plugin)
        }
        eventBuffer.deinitialize(count: Self.maxEvents)
        eventBuffer.deallocate()
        messageBuffer.deinitialize(count: 3)
        messageBuffer.deallocate()
    }

    // -- buses and MIDI ------------------------------------------------------

    public override var inputBusses: AUAudioUnitBusArray { inputBusArray }
    public override var outputBusses: AUAudioUnitBusArray { outputBusArray }

    /// Declaring an output name is what makes the host route our MIDI anywhere. Without
    /// it `MIDIOutputEventBlock` is never populated and the plugin is silent with no
    /// error to explain why.
    public override var midiOutputNames: [String] { ["Unplugged"] }

    public override func allocateRenderResources() throws {
        try super.allocateRenderResources()
        let rate = outputBusArray[0].format.sampleRate
        stateQueue.sync {
            if let plugin { unplugged_plugin_prepare(plugin, rate) }
        }
    }

    // -- state ---------------------------------------------------------------

    /// What the host saves in its session.
    ///
    /// Only which project is open — the project itself lives in the shared directory and
    /// the app owns it. Copying notes in here would give one project two owners that
    /// could disagree, and the host's copy would silently win on reload.
    public override var fullState: [String: Any]? {
        get {
            var dictionary = super.fullState ?? [:]
            stateQueue.sync {
                guard let plugin, let json = unplugged_plugin_state_json(plugin) else { return }
                dictionary["unpluggedState"] = String(cString: json)
                unplugged_plugin_string_free(json)
            }
            return dictionary
        }
        set {
            super.fullState = newValue
            guard let json = newValue?["unpluggedState"] as? String else { return }
            stateQueue.sync {
                guard let plugin else { return }
                _ = json.withCString { unplugged_plugin_set_state_json(plugin, $0) }
            }
        }
    }

    // -- the view's API ------------------------------------------------------

    public func projectsJSON() -> String {
        stateQueue.sync {
            guard let plugin, let json = unplugged_plugin_projects_json(plugin) else { return "[]" }
            defer { unplugged_plugin_string_free(json) }
            return String(cString: json)
        }
    }

    /// Open a project. Returns an error message, or nil on success.
    public func open(projectID: String) -> String? {
        stateQueue.sync {
            guard let plugin else { return "the plugin is not initialised" }
            let code = projectID.withCString { unplugged_plugin_open(plugin, $0) }
            guard code != 0 else { return nil }
            guard let message = unplugged_plugin_last_error(plugin) else {
                return "could not open that project"
            }
            defer { unplugged_plugin_string_free(message) }
            return String(cString: message)
        }
    }

    public func openProjectID() -> String? {
        stateQueue.sync {
            guard let plugin, let json = unplugged_plugin_state_json(plugin) else { return nil }
            defer { unplugged_plugin_string_free(json) }
            guard
                let data = String(cString: json).data(using: .utf8),
                let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
            else { return nil }
            return object["project_id"] as? String
        }
    }

    /// Version, commit and dirty flag of the **running** code.
    ///
    /// The one claim about which build is loaded that cannot be wrong. Everything on disk
    /// describes what is installed; Logic may be running something else entirely.
    public static func buildInfoJSON() -> String {
        guard let json = unplugged_plugin_build_info_json() else { return "{}" }
        defer { unplugged_plugin_string_free(json) }
        return String(cString: json)
    }

    // -- render --------------------------------------------------------------

    public override var internalRenderBlock: AUInternalRenderBlock {
        // Captured once, outside the block: reading `self` on the audio thread would be a
        // retain/release per buffer, and ARC traffic is not real-time safe.
        let plugin = self.plugin
        let events = self.eventBuffer
        let message = self.messageBuffer
        let capacity = UInt32(Self.maxEvents)

        // The block's seven parameters, in order: action flags, timestamp, frame count,
        // output bus number, output data, the realtime event list, and the pull-input
        // block. Getting the last two the wrong way round is easy and does not always
        // fail to compile.
        return { [unowned(unsafe) self] _, timestamp, frameCount, _, outputData, _, _ in
            // Nothing is pulled from the input. A MIDI processor has no audio to fetch,
            // and `AURenderPullInputBlock` has no way to say so — its buffer-list
            // parameter is not optional.
            //
            // The output buffers are cleared rather than left alone: they are not
            // guaranteed silent on arrival, and handing undefined memory back to a host
            // that does mix it is a loud failure in someone's session.
            let buffers = UnsafeMutableAudioBufferListPointer(outputData)
            for index in 0..<buffers.count {
                if let data = buffers[index].mData {
                    memset(data, 0, Int(buffers[index].mDataByteSize))
                }
            }

            guard let plugin else { return noErr }

            // Ask the host where we are. Its answer is authoritative — Rust follows it
            // rather than running a clock of its own. A host that supplies no musical
            // context (rare, but legal) gets a stopped transport rather than a guess.
            var beats: Double = 0
            var tempo: Double = 120
            var playing = false

            if let context = self.musicalContextBlock {
                var timeSignatureNumerator: Double = 4
                var timeSignatureDenominator: Int = 4
                var sampleOffsetToNextBeat: Int = 0
                var currentMeasureDownbeat: Double = 0
                // The order is tempo, numerator, denominator, **beat position**, sample
                // offset, **measure downbeat**. Four of the six are `Double`, so swapping
                // the beat position with the measure downbeat compiles cleanly and then
                // follows the bar line instead of the beat — the part plays, and is
                // quantised to the bar for no visible reason.
                _ = context(
                    &tempo,
                    &timeSignatureNumerator,
                    &timeSignatureDenominator,
                    &beats,
                    &sampleOffsetToNextBeat,
                    &currentMeasureDownbeat
                )
            }

            if let transport = self.transportStateBlock {
                var flags = AUHostTransportStateFlags(rawValue: 0)
                var samplePosition: Double = 0
                var cycleStart: Double = 0
                var cycleEnd: Double = 0
                if transport(&flags, &samplePosition, &cycleStart, &cycleEnd) {
                    playing = flags.contains(.moving)
                }
            }

            let count = unplugged_plugin_render(
                plugin, beats, tempo, playing, frameCount, events, capacity
            )
            guard count > 0, let emit = self.midiOutputEventBlock else { return noErr }

            // Sample time within this buffer. `AUEventSampleTime` is absolute, so the
            // block's own timestamp is the base.
            //
            // Converted defensively: `Int64(someDouble)` traps on a value that is not
            // finite or does not fit, and a trap here does not fail the plugin — it takes
            // the host down with the user's unsaved session. A timestamp that cannot be
            // trusted is better read as zero.
            let sampleTime = timestamp.pointee.mSampleTime
            let base: AUEventSampleTime =
                sampleTime.isFinite && sampleTime.magnitude < 9.0e18
                ? AUEventSampleTime(sampleTime)
                : 0

            for index in 0..<Int(count) {
                let event = events[index]
                // Channel voice message: status nibble plus channel. Velocity 0 would be
                // read as a note-off by some instruments, so a note-off is sent as an
                // explicit 0x80 rather than as a zero-velocity note-on.
                message[0] = (event.kind == 1 ? 0x90 : 0x80) | (event.channel & 0x0F)
                message[1] = event.pitch & 0x7F
                message[2] = event.velocity & 0x7F

                _ = emit(base + AUEventSampleTime(event.frame_offset), 0, 3, message)
            }

            return noErr
        }
    }

    // -- the shared container ------------------------------------------------

    /// Where projects live, shared between the app and this extension.
    ///
    /// Two arrangements, and which one is in play depends only on how the build was signed.
    ///
    /// **Signed** (`--signed`): an App Group. That is the only way a sandboxed extension
    /// and a sandboxed app can see the same files, and it is what ships.
    ///
    /// **Ad-hoc**: no group — it needs a provisioning profile — so both sides use a fixed
    /// path under the real home directory, which the extension reaches through the
    /// read-only sandbox exception in its entitlements. `home_data_dir()` in
    /// src-tauri/src/shared_container.rs writes to the same path, and the entitlement
    /// names it a third time. All three must agree or the project list is silently empty.
    private static func sharedDataDirectory() -> String {
        let group = "group.com.unplugged.daw"
        if let shared = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: group) {
            return shared.appendingPathComponent("Unplugged", isDirectory: true).path
        }
        return homeRelativeDataDirectory()
    }

    /// `~/Library/Application Support/Unplugged`, against the **real** home directory.
    ///
    /// Deliberately not `FileManager.urls(for: .applicationSupportDirectory ...)`, and not
    /// `NSHomeDirectory()`: inside the sandbox both answer with *this extension's own
    /// container*, which is precisely the directory the app cannot write to — so the
    /// project list would be empty and the reason invisible. `getpwuid` reports the
    /// account's home regardless of the container, which is what the sandbox exception is
    /// written against.
    private static func homeRelativeDataDirectory() -> String {
        guard let entry = getpwuid(getuid()), let home = entry.pointee.pw_dir else {
            return NSTemporaryDirectory()
        }
        return String(cString: home) + "/Library/Application Support/Unplugged"
    }
}
