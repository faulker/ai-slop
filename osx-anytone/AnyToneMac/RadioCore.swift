import Foundation

/// Errors surfaced from the Rust core across the C FFI boundary.
enum RadioCoreError: LocalizedError {
    case native(String)

    var errorDescription: String? {
        switch self {
        case .native(let message): return message
        }
    }
}

/// Thin Swift wrapper over the anytone-core C FFI (`core/src/ffi.rs`,
/// `core/include/anytone_core.h`). All calls are synchronous and blocking;
/// run device operations off the main thread.
enum RadioCore {

    /// Box carried through the C progress callback's `user` pointer.
    private final class ProgressBox {
        let handler: (Int, Int) -> Void
        init(_ handler: @escaping (Int, Int) -> Void) { self.handler = handler }
    }

    /// C trampoline: forwards (done, total) to the boxed Swift closure.
    private static let progressTrampoline: anytone_progress_cb = { done, total, user in
        guard let user = user else { return }
        Unmanaged<ProgressBox>.fromOpaque(user).takeUnretainedValue().handler(Int(done), Int(total))
    }

    /// Take ownership of a Rust-allocated C string, copy it, and free it.
    private static func consume(_ ptr: UnsafeMutablePointer<CChar>?) -> String? {
        guard let ptr = ptr else { return nil }
        defer { anytone_string_free(ptr) }
        return String(cString: ptr)
    }

    /// Build the thrown error from an `err_out` pointer, freeing it.
    private static func nativeError(_ err: UnsafeMutablePointer<CChar>?) -> RadioCoreError {
        .native(consume(err) ?? "unknown native error")
    }

    /// Decode JSON produced by the core into a Codable model.
    private static func decode<T: Decodable>(_ type: T.Type, from json: String) throws -> T {
        do {
            return try JSONDecoder().decode(type, from: Data(json.utf8))
        } catch {
            throw RadioCoreError.native("failed to decode core JSON: \(error)")
        }
    }

    /// The exact byte length this build expects a full codeplug .bin to be. A
    /// file of any other length is from a different radio model or firmware
    /// version and can't be parsed or written; callers use this to reject an
    /// incompatible file with a clear message before touching it.
    static var expectedCodeplugSize: Int { anytone_codeplug_size() }

    /// Enumerate serial ports, flagging likely radios.
    static func listPorts() throws -> [PortEntry] {
        var err: UnsafeMutablePointer<CChar>? = nil
        guard let json = consume(anytone_ports_json(&err)) else { throw nativeError(err) }
        return try decode([PortEntry].self, from: json)
    }

    /// Identify the radio on `port`; returns its model/version string.
    static func identify(port: String) throws -> String {
        var err: UnsafeMutablePointer<CChar>? = nil
        guard let model = consume(anytone_identify(port, &err)) else { throw nativeError(err) }
        return model
    }

    /// Read the full codeplug from the radio into the file at `path`.
    /// `progress` is called on the calling (background) thread.
    static func backup(port: String, to path: String, progress: @escaping (Int, Int) -> Void) throws {
        var err: UnsafeMutablePointer<CChar>? = nil
        let box = ProgressBox(progress)
        let user = Unmanaged.passRetained(box).toOpaque()
        defer { Unmanaged<ProgressBox>.fromOpaque(user).release() }
        if anytone_backup(port, path, progressTrampoline, user, &err) != 0 {
            throw nativeError(err)
        }
    }

    /// Write the codeplug file at `path` back to the radio. `force` must be
    /// true or the core refuses (same gate as the CLI); the core also checks
    /// the model string and read-back verifies every block.
    static func restore(port: String, from path: String, force: Bool,
                        progress: @escaping (Int, Int) -> Void) throws {
        var err: UnsafeMutablePointer<CChar>? = nil
        let box = ProgressBox(progress)
        let user = Unmanaged.passRetained(box).toOpaque()
        defer { Unmanaged<ProgressBox>.fromOpaque(user).release() }
        if anytone_restore(port, path, force, progressTrampoline, user, &err) != 0 {
            throw nativeError(err)
        }
    }

    /// Parse a codeplug .bin offline into channels and zones.
    static func dump(binPath: String) throws -> CodeplugDump {
        var err: UnsafeMutablePointer<CChar>? = nil
        guard let json = consume(anytone_dump_json(binPath, &err)) else { throw nativeError(err) }
        return try decode(CodeplugDump.self, from: json)
    }

    /// Apply a batch of edits (channels, zones, contacts, group lists, radio
    /// IDs; add/remove/update) to `input`, writing the result to `output` (the
    /// paths may be equal). Edits round-trip through the Rust Codeplug model and
    /// are verified by re-parsing.
    static func applyEdits(input: String, spec: EditSpec, output: String) throws {
        let data: Data
        do {
            data = try JSONEncoder().encode(spec)
        } catch {
            throw RadioCoreError.native("failed to encode edits JSON: \(error)")
        }
        let json = String(decoding: data, as: UTF8.self)
        var err: UnsafeMutablePointer<CChar>? = nil
        if anytone_apply_edits(input, json, output, &err) != 0 {
            throw nativeError(err)
        }
    }
}
