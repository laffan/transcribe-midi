import SwiftUI

/// The container app the extension ships inside.
///
/// Deliberately almost nothing. macOS discovers an AUv3 by scanning *apps*, not plugin
/// folders, so an extension needs a host bundle to live in — but the real Unplugged app is
/// the Tauri build, and the intent is for the extension to be embedded there once that
/// build learns to embed one. Until then this exists so the plugin can be built,
/// installed and loaded on its own without waiting on that.
///
/// It says what it is rather than pretending to be the app, so nobody spends ten minutes
/// wondering why the editor will not open.
///
/// Not in a file called `main.swift`: Swift treats that one filename as top-level code,
/// where `@main` is rejected outright. The name of the file is the whole difference.
@main
struct UnpluggedAUHostApp: App {
    var body: some Scene {
        WindowGroup("Unplugged AU") {
            VStack(alignment: .leading, spacing: 12) {
                Text("Unplugged — Audio Unit").font(.headline)
                Text(
                    "This window exists only so macOS registers the Audio Unit inside it. "
                        + "Open your DAW and add Unplugged as a MIDI effect."
                )
                .font(.callout)
                .foregroundStyle(.secondary)
                Text(
                    "Editing lives in the Unplugged app; the plugin plays what the app "
                        + "authored. They share a projects folder."
                )
                .font(.caption)
                .foregroundStyle(.secondary)
                Spacer()
            }
            .padding(20)
            .frame(width: 420, height: 200)
        }
        .windowResizability(.contentSize)
    }
}
