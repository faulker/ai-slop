package me.faulk.aetr

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
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
 */
class MainActivity : ComponentActivity() {

    private val vm: AetrViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            MaterialTheme(
                colorScheme = if (isSystemInDarkTheme()) darkColorScheme()
                else lightColorScheme()
            ) {
                AetrScreen(vm)
            }
        }
    }
}
