import CoreAudioKit
import SwiftUI

#if os(macOS)
import AppKit
public typealias PlatformViewController = NSViewController
#else
import UIKit
public typealias PlatformViewController = UIViewController
#endif

/// The plugin's view.
///
/// **Deliberately native and small for this first pass.** The eventual plan is the React
/// editor in a `WKWebView` — the frontend is ready for it, since `src/lib/api.ts` is the
/// only place it talks to Rust — but that needs the whole Tauri command surface re-homed
/// behind a bridge that is not Tauri. Doing that in the same step as getting an extension
/// to load at all would mean two unverified things failing together, and no way to tell
/// which one broke.
///
/// So this pass answers three questions and nothing else: does the plugin load, which
/// build is it, and which project is it playing.
public final class UnpluggedAUViewController: AUViewControllerBase, AUAudioUnitFactory {
    private var unit: UnpluggedAudioUnit?
    private var model = PluginViewModel()

    public override func viewDidLoad() {
        super.viewDidLoad()
        embed()
    }

    /// Called by the host to make the Audio Unit.
    ///
    /// May arrive before *or* after `viewDidLoad` — hosts differ, and Logic has done both
    /// — so each side wires up whatever the other has already produced.
    public func createAudioUnit(with componentDescription: AudioComponentDescription) throws
        -> AUAudioUnit
    {
        let unit = try UnpluggedAudioUnit(componentDescription: componentDescription)
        self.unit = unit
        DispatchQueue.main.async { [weak self] in
            self?.model.attach(unit)
        }
        return unit
    }

    private func embed() {
        let root = NSHostingController(rootView: PluginView(model: model))
        addChild(root)
        root.view.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(root.view)
        NSLayoutConstraint.activate([
            root.view.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            root.view.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            root.view.topAnchor.constraint(equalTo: view.topAnchor),
            root.view.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])
        #if os(macOS)
        root.didMove(toParent: self)
        #else
        root.didMove(toParent: self)
        #endif

        if let unit { model.attach(unit) }
    }
}

// ---------------------------------------------------------------------------

struct PluginProject: Identifiable, Decodable {
    let id: String
    let name: String
    let tempo_bpm: Double
    let track_count: Int
}

struct BuildStamp: Decodable {
    let version: String
    let commit: String
    let dirty: Bool
    let built_at: String
    let profile: String

    /// What the window shows. The `+` is the load-bearing character: it is the difference
    /// between "this is that commit" and "this is something like that commit".
    var short: String { "\(version) \(commit)\(dirty ? "+" : "")" }
}

@MainActor
final class PluginViewModel: ObservableObject {
    @Published var projects: [PluginProject] = []
    @Published var selected: String?
    @Published var error: String?
    @Published var build: BuildStamp?

    private weak var unit: UnpluggedAudioUnit?

    init() {
        build = try? JSONDecoder().decode(
            BuildStamp.self,
            from: Data(UnpluggedAudioUnit.buildInfoJSON().utf8)
        )
    }

    func attach(_ unit: UnpluggedAudioUnit) {
        self.unit = unit
        refresh()
        selected = unit.openProjectID()
    }

    func refresh() {
        guard let unit else { return }
        projects = (try? JSONDecoder().decode(
            [PluginProject].self,
            from: Data(unit.projectsJSON().utf8)
        )) ?? []
    }

    func open(_ id: String) {
        guard let unit else { return }
        error = unit.open(projectID: id)
        if error == nil { selected = id }
    }
}

struct PluginView: View {
    @ObservedObject var model: PluginViewModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("Unplugged").font(.headline)
                Spacer()
                if let build = model.build {
                    // Always visible, never behind a disclosure. In a host this is the
                    // only reliable answer to "am I running the build I just made?" —
                    // Logic caches AU scans and keeps extensions alive across replacement.
                    Text(build.short)
                        .font(.system(.caption, design: .monospaced))
                        .foregroundStyle(build.dirty ? .orange : .secondary)
                        .help("\(build.profile) · built \(build.built_at)")
                }
            }

            if model.projects.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    Text("No projects found.").font(.callout)
                    Text(
                        "Create one in the Unplugged app. The plugin plays what the app "
                            + "authored; they share a projects folder."
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }
            } else {
                Text("Project").font(.caption).foregroundStyle(.secondary)
                Picker("", selection: Binding(
                    get: { model.selected ?? "" },
                    set: { if !$0.isEmpty { model.open($0) } }
                )) {
                    Text("None").tag("")
                    ForEach(model.projects) { project in
                        Text("\(project.name) — \(project.track_count) tracks").tag(project.id)
                    }
                }
                .labelsHidden()
                .pickerStyle(.menu)
            }

            if let error = model.error {
                Text(error).font(.caption).foregroundStyle(.red)
            }

            Text("Follows the host transport and sends MIDI. Put an instrument after it.")
                .font(.caption)
                .foregroundStyle(.secondary)

            Spacer()

            Button("Refresh") { model.refresh() }
        }
        .padding(16)
        .frame(minWidth: 380, minHeight: 220)
    }
}
