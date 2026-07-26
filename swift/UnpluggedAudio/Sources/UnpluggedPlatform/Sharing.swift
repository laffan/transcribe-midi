import Foundation

#if os(iOS)
import UIKit
#else
import AppKit
import UniformTypeIdentifiers
#endif

/// Phase 5 platform surface: share sheet, pasteboard and drag-out.
///
/// Same C-ABI shape as the audio target and for the same reason — one Swift
/// implementation, linked into both binaries, called only from Rust. Everything here
/// takes a path to a file Rust has already staged; deciding *what* the bytes are is
/// Rust's job, and putting them somewhere the user can reach is this file's.
///
/// All entry points return 0 on success and non-zero on failure.

private func pathString(_ pointer: UnsafePointer<CChar>?) -> String? {
    guard let pointer else { return nil }
    let path = String(cString: pointer)
    return path.isEmpty ? nil : path
}

/// Put a `.mid` on the pasteboard as both a file URL and raw data.
///
/// Note what this does and does not do, because the distinction matters and the UI says
/// so explicitly: this pastes a *file* into Finder or Files. It does not paste notes
/// into Logic's piano roll — Logic's note clipboard format is proprietary and
/// undocumented, and the spec rules out reverse-engineering it.
@_cdecl("unplugged_platform_copy_file")
public func unplugged_platform_copy_file(_ cPath: UnsafePointer<CChar>?) -> Int32 {
    guard let path = pathString(cPath) else { return -1 }
    let url = URL(fileURLWithPath: path)
    guard let data = try? Data(contentsOf: url) else { return 1 }

    #if os(iOS)
    let pasteboard = UIPasteboard.general
    // "public.midi-audio" is the UTI the spec names; declaring both it and the file URL
    // lets Files accept the paste as a file while other apps can take the bytes.
    pasteboard.items = [[
        "public.midi-audio": data,
        UTType.fileURL.identifier: url.dataRepresentation,
    ]]
    return 0
    #else
    let pasteboard = NSPasteboard.general
    pasteboard.clearContents()
    let item = NSPasteboardItem()
    item.setData(data, forType: NSPasteboard.PasteboardType("public.midi-audio"))
    guard pasteboard.writeObjects([url as NSURL]) else { return 1 }
    pasteboard.writeObjects([item])
    return 0
    #endif
}

#if os(iOS)

/// Present the system share sheet for a staged file.
///
/// `UIActivityViewController` needs a presenting controller and, on iPad, a source rect
/// for the popover — omitting the latter is a hard crash there, not a cosmetic problem,
/// so a centred fallback anchor is always supplied.
@_cdecl("unplugged_platform_share_file")
public func unplugged_platform_share_file(_ cPath: UnsafePointer<CChar>?) -> Int32 {
    guard let path = pathString(cPath) else { return -1 }
    let url = URL(fileURLWithPath: path)

    var status: Int32 = 0
    let present = {
        guard let root = keyRootViewController() else {
            status = 2
            return
        }

        let controller = UIActivityViewController(activityItems: [url], applicationActivities: nil)
        if let popover = controller.popoverPresentationController {
            popover.sourceView = root.view
            popover.sourceRect = CGRect(
                x: root.view.bounds.midX,
                y: root.view.bounds.midY,
                width: 0,
                height: 0
            )
            popover.permittedArrowDirections = []
        }
        root.present(controller, animated: true)
    }

    if Thread.isMainThread {
        present()
    } else {
        DispatchQueue.main.sync(execute: present)
    }
    return status
}

private func keyRootViewController() -> UIViewController? {
    let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
    let window = scenes
        .flatMap(\.windows)
        .first(where: \.isKeyWindow) ?? scenes.flatMap(\.windows).first

    var controller = window?.rootViewController
    while let presented = controller?.presentedViewController {
        controller = presented
    }
    return controller
}

#else

/// Begin a native drag of a staged file from the app window.
///
/// This is what makes a track droppable into Logic Pro or GarageBand: they accept a
/// dropped SMF, but a drag originating inside WKWebView's HTML5 drag API does not carry
/// a real file promise, so the drag has to start from AppKit.
@_cdecl("unplugged_platform_begin_file_drag")
public func unplugged_platform_begin_file_drag(_ cPath: UnsafePointer<CChar>?) -> Int32 {
    guard let path = pathString(cPath) else { return -1 }
    let url = URL(fileURLWithPath: path)
    guard FileManager.default.fileExists(atPath: path) else { return 1 }

    var status: Int32 = 0
    let start = {
        guard let window = NSApplication.shared.keyWindow ?? NSApplication.shared.windows.first,
              let view = window.contentView
        else {
            status = 2
            return
        }

        // A drag must be driven by a real event. Without one there is nothing to attach
        // the session to, so this is reported rather than faked.
        guard let event = NSApplication.shared.currentEvent else {
            status = 3
            return
        }

        let item = NSDraggingItem(pasteboardWriter: url as NSURL)
        let size = NSSize(width: 96, height: 96)
        let origin = view.convert(event.locationInWindow, from: nil)
        let frame = NSRect(
            x: origin.x - size.width / 2,
            y: origin.y - size.height / 2,
            width: size.width,
            height: size.height
        )

        // The Finder icon for the file is the most legible drag image available without
        // shipping artwork.
        let icon = NSWorkspace.shared.icon(forFile: path)
        item.setDraggingFrame(frame, contents: icon)

        view.beginDraggingSession(with: [item], event: event, source: FileDragSource.shared)
    }

    if Thread.isMainThread {
        start()
    } else {
        DispatchQueue.main.sync(execute: start)
    }
    return status
}

/// Drag source for exported files.
///
/// `.copy` for destinations outside the app so Logic imports rather than moves the file,
/// and `[]` inside the app so a drag that never leaves the window does nothing.
private final class FileDragSource: NSObject, NSDraggingSource {
    static let shared = FileDragSource()

    func draggingSession(
        _ session: NSDraggingSession,
        sourceOperationMaskFor context: NSDraggingContext
    ) -> NSDragOperation {
        switch context {
        case .outsideApplication: return .copy
        case .withinApplication: return []
        @unknown default: return .copy
        }
    }
}

#endif
