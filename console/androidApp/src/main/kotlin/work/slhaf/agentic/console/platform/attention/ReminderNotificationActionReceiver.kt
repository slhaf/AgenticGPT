package work.slhaf.agentic.console.platform.attention

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch

class ReminderNotificationActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val pendingResult = goAsync()
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        scope.launch {
            try {
                AttentionRuntimeCoordinator(context, scope).handleNotificationIntent(intent)
            } catch (_: Throwable) {
                // Broadcast work must not crash the app process.
            } finally {
                pendingResult.finish()
                scope.cancel()
            }
        }
    }

    companion object {
        const val ACTION_DONE = "work.slhaf.agentic.console.action.DONE"
        const val ACTION_ACKNOWLEDGE = "work.slhaf.agentic.console.action.ACKNOWLEDGE"
        const val ACTION_SNOOZE = "work.slhaf.agentic.console.action.SNOOZE"
        const val ACTION_SHOW_SNOOZED_NOTIFICATION = "work.slhaf.agentic.console.action.SHOW_SNOOZED_NOTIFICATION"
        const val ACTION_FIRE_ATTENTION_ITEM = "work.slhaf.agentic.console.action.FIRE_ATTENTION_ITEM"
    }
}
