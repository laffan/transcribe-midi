import AVFoundation
import Foundation

/// Decoding an audio file to mono float samples, for transcription.
///
/// The microphone is not the main way audio arrives here. A voice memo, a bounce, a stem
/// someone sent — and, once this is a plugin, whatever the host hands over — are all more
/// likely than someone humming into a laptop. This is that path.
///
/// `AVAudioFile` reads anything CoreAudio can open: wav, aiff, caf, m4a, mp3, and the
/// Voice Memos format. Formats are deliberately not enumerated here; the system decides
/// what it can decode and reports what it cannot.
///
/// Nothing is retained: the caller gets a buffer and owns it.

/// Longest file accepted, in seconds. Matches the live capture limit for the same reason
/// — beyond a couple of minutes this stops being a phrase and the analysis stops being
/// something a person can review note by note.
private let maxSeconds = 120.0

private enum DecodeError: Int32 {
    case badArguments = -1
    case unreadable = 1
    case unsupported = 2
    case tooLong = 3
    case empty = 4
}

/// Decode to mono `Float`, at the file's own sample rate.
///
/// Returns a malloc'd buffer the caller frees with `unplugged_audio_free_samples`, or
/// NULL on failure with the reason in `outStatus`.
@_cdecl("unplugged_audio_decode_file")
public func unplugged_audio_decode_file(
    _ cPath: UnsafePointer<CChar>?,
    _ outCount: UnsafeMutablePointer<UInt32>?,
    _ outRate: UnsafeMutablePointer<Double>?,
    _ outStatus: UnsafeMutablePointer<Int32>?
) -> UnsafeMutablePointer<Float>? {
    func fail(_ error: DecodeError) -> UnsafeMutablePointer<Float>? {
        outStatus?.pointee = error.rawValue
        return nil
    }

    guard let cPath, let outCount, let outRate else { return fail(.badArguments) }
    let path = String(cString: cPath)
    if path.isEmpty { return fail(.badArguments) }

    let url = URL(fileURLWithPath: path)
    guard let file = try? AVAudioFile(forReading: url) else {
        return fail(.unreadable)
    }

    let sourceRate = file.fileFormat.sampleRate
    let frames = file.length
    guard sourceRate > 0, frames > 0 else { return fail(.empty) }
    guard Double(frames) / sourceRate <= maxSeconds else { return fail(.tooLong) }

    // Read as deinterleaved 32-bit float at the file's own rate. `AVAudioFile` converts
    // to this processing format for us, so an mp3 or an AAC voice memo arrives in the
    // same shape as a wav and nothing downstream has to care which it was.
    guard
        let format = AVAudioFormat(
            commonFormat: .pcmFormatFloat32,
            sampleRate: sourceRate,
            channels: file.fileFormat.channelCount,
            interleaved: false
        ),
        let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(frames))
    else {
        return fail(.unsupported)
    }

    do {
        try file.read(into: buffer)
    } catch {
        return fail(.unreadable)
    }

    let count = Int(buffer.frameLength)
    guard count > 0, let channels = buffer.floatChannelData else { return fail(.empty) }
    let channelCount = Int(format.channelCount)

    guard let out = malloc(count * MemoryLayout<Float>.size)?.assumingMemoryBound(to: Float.self)
    else {
        return fail(.unsupported)
    }

    // Downmix. Transcription is monophonic; a stereo pair of the same source would only
    // double the work per sample.
    let scale = 1.0 / Float(max(1, channelCount))
    for frame in 0..<count {
        var sum: Float = 0
        for channel in 0..<channelCount {
            sum += channels[channel][frame]
        }
        out[frame] = sum * scale
    }

    outCount.pointee = UInt32(count)
    outRate.pointee = sourceRate
    outStatus?.pointee = 0
    return out
}

@_cdecl("unplugged_audio_free_samples")
public func unplugged_audio_free_samples(_ pointer: UnsafeMutablePointer<Float>?) {
    guard let pointer else { return }
    free(pointer)
}
