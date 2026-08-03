package me.faulk.aetr

import android.content.Intent
import android.media.session.MediaSession
import android.os.Bundle
import android.view.KeyEvent
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import me.faulk.aetr.ui.AetrScreen

/**
 * Hosts the single Compose screen. All state lives in [AetrViewModel] so it
 * survives rotation; audio + session teardown happens in the ViewModel's
 * onCleared.
 *
 * Also swallows wired-headset media/assist keys. APRS TRRS and many USB-C
 * audio cables float the mic-button line; Android treats that as
 * HEADSETHOOK / voice-assist and launches Gemini mid TX/RX.
 */
class MainActivity : ComponentActivity() {

    private val vm: AetrViewModel by viewModels()
    private var mediaSession: MediaSession? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        // Draw behind system bars; Compose applies status/nav insets so the
        // header is not covered by the status / menu bar on modern Android.
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        mediaSession = MediaSession(this, "aetr").apply {
            setCallback(
                object : MediaSession.Callback() {
                    /** Eat headset media-button intents so Assistant does not launch. */
                    override fun onMediaButtonEvent(mediaButtonIntent: Intent): Boolean = true
                }
            )
            isActive = true
        }
        setContent {
            MaterialTheme(
                colorScheme = if (isSystemInDarkTheme()) darkColorScheme()
                else lightColorScheme()
            ) {
                AetrScreen(vm)
            }
        }
    }

    override fun onDestroy() {
        mediaSession?.isActive = false
        mediaSession?.release()
        mediaSession = null
        super.onDestroy()
    }

    /** Drops headset hook / assist keys that APRS cables spuriously generate. */
    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        if (isHeadsetAssistKey(event.keyCode)) return true
        return super.dispatchKeyEvent(event)
    }

    override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean {
        if (isHeadsetAssistKey(keyCode)) return true
        return super.onKeyDown(keyCode, event)
    }

    override fun onKeyUp(keyCode: Int, event: KeyEvent): Boolean {
        if (isHeadsetAssistKey(keyCode)) return true
        return super.onKeyUp(keyCode, event)
    }

    companion object {
        /** Keys commonly synthesized from TRRS mic-button / PTT wiring noise. */
        fun isHeadsetAssistKey(keyCode: Int): Boolean = when (keyCode) {
            KeyEvent.KEYCODE_HEADSETHOOK,
            KeyEvent.KEYCODE_MEDIA_PLAY_PAUSE,
            KeyEvent.KEYCODE_MEDIA_PLAY,
            KeyEvent.KEYCODE_MEDIA_PAUSE,
            KeyEvent.KEYCODE_MEDIA_STOP,
            KeyEvent.KEYCODE_VOICE_ASSIST,
            KeyEvent.KEYCODE_ASSIST,
            -> true
            else -> false
        }
    }
}
