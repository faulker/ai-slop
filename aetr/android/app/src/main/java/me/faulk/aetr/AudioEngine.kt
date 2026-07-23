package me.faulk.aetr

import android.annotation.SuppressLint
import android.media.AudioAttributes
import android.media.AudioDeviceInfo
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.AudioTrack
import android.media.MediaRecorder
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors

/**
 * Owns both halves of the audio path at the rates the core expects:
 * a 48 kHz mono float AudioRecord capture loop that hands every PCM block to
 * a callback (which the ViewModel feeds into `AetrSession.pushRx`), and a
 * 48 kHz mono float AudioTrack for playing encoded TX bursts and received
 * voice clips. Playback requests are serialized on a single worker thread so
 * bursts never overlap on air.
 */
class AudioEngine {

    companion object {
        const val SAMPLE_RATE = 48_000

        /** Frames per capture read; ~21 ms at 48 kHz. */
        private const val CAPTURE_BLOCK = 1024
    }

    // --- Device routing (set by the ViewModel before startCapture) ---
    // Both stay at 48 kHz mono float: the framework resamples to the link
    // rate, and SCO is narrowband anyway; the modem lives in 300-3000 Hz so
    // that survives. Note that lossy Bluetooth codecs can still degrade the
    // denser modem modes (128/170 B) enough to cost decodes; the UI
    // recommends the robust 85 B mode when a Bluetooth device is active.

    /** Capture device to pin via setPreferredDevice (null = system default). */
    @Volatile
    var preferredInput: AudioDeviceInfo? = null

    /** Playback device to pin via setPreferredDevice (null = system default). */
    @Volatile
    var preferredOutput: AudioDeviceInfo? = null

    /**
     * When true (a Bluetooth SCO route is active), capture uses
     * VOICE_COMMUNICATION and playback uses USAGE_VOICE_COMMUNICATION so the
     * streams follow the communication route the platform was switched to.
     */
    @Volatile
    var communicationRoute = false

    private var record: AudioRecord? = null
    private var captureThread: Thread? = null

    @Volatile
    private var capturing = false

    private val playExecutor: ExecutorService = Executors.newSingleThreadExecutor { r ->
        Thread(r, "aetr-playback")
    }

    @Volatile
    private var currentTrack: AudioTrack? = null

    /**
     * Starts the mic capture loop. `onPcm` is invoked on the capture thread
     * with each freshly read block; it must be cheap (the core's push_rx is).
     * Requires RECORD_AUDIO to already be granted (lint suppressed because
     * the caller enforces it before connecting).
     */
    @SuppressLint("MissingPermission")
    fun startCapture(onPcm: (FloatArray) -> Unit) {
        if (capturing) return
        val minBuf = AudioRecord.getMinBufferSize(
            SAMPLE_RATE,
            AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_FLOAT
        )
        val rec = AudioRecord(
            if (communicationRoute) MediaRecorder.AudioSource.VOICE_COMMUNICATION
            else MediaRecorder.AudioSource.VOICE_RECOGNITION,
            SAMPLE_RATE,
            AudioFormat.CHANNEL_IN_MONO,
            AudioFormat.ENCODING_PCM_FLOAT,
            maxOf(minBuf * 4, CAPTURE_BLOCK * 4 * java.lang.Float.BYTES)
        )
        check(rec.state == AudioRecord.STATE_INITIALIZED) {
            "AudioRecord failed to initialize (48 kHz mono float unsupported?)"
        }
        preferredInput?.let { rec.preferredDevice = it }
        record = rec
        capturing = true
        rec.startRecording()
        captureThread = Thread({
            val buf = FloatArray(CAPTURE_BLOCK)
            while (capturing) {
                val n = rec.read(buf, 0, buf.size, AudioRecord.READ_BLOCKING)
                if (n > 0) onPcm(buf.copyOf(n))
            }
        }, "aetr-capture").also { it.start() }
    }

    /** Stops the capture loop and releases the AudioRecord. */
    fun stopCapture() {
        capturing = false
        captureThread?.join(1000)
        captureThread = null
        record?.let {
            runCatching { it.stop() }
            it.release()
        }
        record = null
    }

    /**
     * Queues a PCM burst for playback. Bursts play back-to-back in submission
     * order; `onDone` fires (on the playback thread) after the burst drains.
     */
    fun play(pcm: FloatArray, onDone: (() -> Unit)? = null) {
        if (pcm.isEmpty()) {
            onDone?.invoke()
            return
        }
        playExecutor.execute {
            try {
                playBlocking(pcm)
            } finally {
                onDone?.invoke()
            }
        }
    }

    /**
     * Writes one burst through a stream-mode AudioTrack and blocks until the
     * track has actually played it out (stop() in stream mode drains).
     */
    private fun playBlocking(pcm: FloatArray) {
        val minBuf = AudioTrack.getMinBufferSize(
            SAMPLE_RATE,
            AudioFormat.CHANNEL_OUT_MONO,
            AudioFormat.ENCODING_PCM_FLOAT
        )
        val track = AudioTrack.Builder()
            .setAudioAttributes(
                AudioAttributes.Builder()
                    .setUsage(
                        if (communicationRoute) AudioAttributes.USAGE_VOICE_COMMUNICATION
                        else AudioAttributes.USAGE_MEDIA
                    )
                    .setContentType(
                        if (communicationRoute) AudioAttributes.CONTENT_TYPE_SPEECH
                        else AudioAttributes.CONTENT_TYPE_MUSIC
                    )
                    .build()
            )
            .setAudioFormat(
                AudioFormat.Builder()
                    .setEncoding(AudioFormat.ENCODING_PCM_FLOAT)
                    .setSampleRate(SAMPLE_RATE)
                    .setChannelMask(AudioFormat.CHANNEL_OUT_MONO)
                    .build()
            )
            .setTransferMode(AudioTrack.MODE_STREAM)
            .setBufferSizeInBytes(minBuf * 4)
            .build()
        preferredOutput?.let { track.preferredDevice = it }
        currentTrack = track
        try {
            track.play()
            var offset = 0
            while (offset < pcm.size) {
                val n = track.write(
                    pcm, offset, pcm.size - offset, AudioTrack.WRITE_BLOCKING
                )
                if (n <= 0) break
                offset += n
            }
            // In MODE_STREAM stop() lets the already-written audio drain;
            // poll the head position until it reaches the end.
            track.stop()
            val deadline = System.currentTimeMillis() + 2000
            while (track.playbackHeadPosition < pcm.size &&
                track.playState != AudioTrack.PLAYSTATE_STOPPED &&
                System.currentTimeMillis() < deadline
            ) {
                Thread.sleep(20)
            }
        } finally {
            currentTrack = null
            track.release()
        }
    }

    /** Cuts any burst that is currently playing. */
    fun stopPlayback() {
        currentTrack?.let { runCatching { it.pause(); it.flush() } }
    }

    /** Releases everything; the engine is unusable afterwards. */
    fun shutdown() {
        stopCapture()
        stopPlayback()
        playExecutor.shutdownNow()
    }
}
