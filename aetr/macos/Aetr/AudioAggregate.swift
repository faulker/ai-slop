import CoreAudio
import Foundation

/// Creates and tears down a private CoreAudio aggregate device.
///
/// An `AVAudioEngine` on macOS is backed by a single hardware I/O unit, so it
/// can only drive one physical device for both capture and playback. When the
/// user picks a different input and output device (for example a built-in mic
/// plus a USB-C output dongle) we bridge them into one private aggregate device
/// and point the engine at that instead. "Private" means it is visible only to
/// this process and is not persisted in the user's device list.
enum AudioAggregate {
    /// The UID of a device, needed to reference it as an aggregate sub-device.
    private static func uid(_ device: AudioDeviceID) -> String? {
        var addr = AudioObjectPropertyAddress(
            mSelector: kAudioDevicePropertyDeviceUID,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain)
        var cf: CFString?
        var size = UInt32(MemoryLayout<CFString?>.size)
        let status = withUnsafeMutablePointer(to: &cf) { ptr in
            AudioObjectGetPropertyData(device, &addr, 0, nil, &size, ptr)
        }
        guard status == noErr, let cf else { return nil }
        return cf as String
    }

    /// Builds a private aggregate spanning `input` (as capture) and `output`
    /// (as playback, and the clock master). Returns the new aggregate device id,
    /// or nil if either UID is unavailable or creation fails. Destroy the result
    /// with `destroy(_:)` when done.
    static func create(input: AudioDeviceID, output: AudioDeviceID) -> AudioDeviceID? {
        guard let inputUID = uid(input), let outputUID = uid(output) else { return nil }

        let aggregateUID = "me.faulk.aetr.aggregate.\(UUID().uuidString)"
        let subDevices: [[String: Any]] = [
            [kAudioSubDeviceUIDKey as String: inputUID],
            [kAudioSubDeviceUIDKey as String: outputUID],
        ]
        let description: [String: Any] = [
            kAudioAggregateDeviceNameKey as String: "Aetr Aggregate",
            kAudioAggregateDeviceUIDKey as String: aggregateUID,
            kAudioAggregateDeviceIsPrivateKey as String: 1,
            kAudioAggregateDeviceIsStackedKey as String: 0,
            // The output device drives the clock so playback stays glitch-free;
            // the input sub-device is rate-converted to it by the HAL.
            kAudioAggregateDeviceMasterSubDeviceKey as String: outputUID,
            kAudioAggregateDeviceSubDeviceListKey as String: subDevices,
        ]

        var aggregate: AudioDeviceID = kAudioObjectUnknown
        let status = AudioHardwareCreateAggregateDevice(description as CFDictionary, &aggregate)
        guard status == noErr, aggregate != kAudioObjectUnknown else { return nil }
        return aggregate
    }

    /// Destroys an aggregate device previously returned by `create`.
    static func destroy(_ device: AudioDeviceID) {
        guard device != kAudioObjectUnknown else { return }
        AudioHardwareDestroyAggregateDevice(device)
    }
}
