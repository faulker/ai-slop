package me.faulk.anytone

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.viewModels
import me.faulk.anytone.state.AppViewModel
import me.faulk.anytone.ui.CodeplugApp
import me.faulk.anytone.ui.theme.CodeplugTheme

/**
 * The app's single activity.
 *
 * Deliberately thin: a codeplug transfer belongs to a foreground service, not to
 * an activity, so nothing here needs to survive anything. See
 * `usb/RadioTransferService.kt` for why.
 */
class MainActivity : ComponentActivity() {

    private val viewModel: AppViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            CodeplugTheme {
                CodeplugApp(viewModel)
            }
        }
    }

    /**
     * Re-scan on resume so a cable plugged in while the app was backgrounded
     * shows up without the user hunting for a refresh button.
     */
    override fun onResume() {
        super.onResume()
        viewModel.refreshDevices()
    }
}
