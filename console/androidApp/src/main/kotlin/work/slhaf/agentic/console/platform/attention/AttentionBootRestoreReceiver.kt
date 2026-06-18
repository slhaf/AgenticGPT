package work.slhaf.agentic.console.platform.attention

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

class AttentionBootRestoreReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_BOOT_COMPLETED) return

        val pendingResult = goAsync()
        CoroutineScope(SupervisorJob() + Dispatchers.IO).launch {
            try {
                AttentionRuntimeCoordinator(context).restoreFutureItems()
            } catch (_: Throwable) {
                // Boot restore is best-effort and must not crash the app process.
            } finally {
                pendingResult.finish()
            }
        }
    }
}
