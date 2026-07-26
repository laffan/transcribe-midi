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
        let capacity = UInt32(Self.maxEvents)

        return { [unowned(unsafe) self] actionFlags, timestamp, frameCount, _, _, pullInput, _ in
            // A MIDI processor still has to pull its input so the chain stays connected.
            if let pullInput {
                var flags = actionFlags.pointee
                _ = pullInput(&flags, timestamp, frameCount, 0, nil)
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
                var currentMeasureDownbeat: Double = 0
                var sampleOffsetToNextBeat: Int = 0
                _ = context(
                    &tempo,
                    &timeSignatureNumerator,
                    &timeSignatureDenominator,
                    &currentMeasureDownbeat,
                    &sampleOffsetToNextBeat,
                    &beats
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

            for index in 0..<Int(count) {
                let event = events[index]
                // Channel voice message: status nibble plus channel. Velocity 0 would be
                // read as a note-off by some instruments, so a note-off is sent as an
                // explicit 0x80 rather than as a zero-velocity note-on.
                let status: UInt8 = (event.kind == 1 ? 0x90 : 0x80) | (event.channel & 0x0F)
                var bytes: [UInt8] = [status, event.pitch & 0x7F, event.velocity & 0x7F]

                // Sample time within this buffer. `AUEventSampleTime` is absolute, so the
                // block's own timestamp is the base.
                let when = AUEventSampleTime(timestamp.pointee.mSampleTime)
                    + AUEventSampleTime(event.frame_offset)
                _ = emit(when, 0, bytes.count, &bytes)
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
