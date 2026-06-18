package work.slhaf.agentic.console.platform.attention

import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import work.slhaf.agentic.console.domain.attention.AttentionType

class AlarmActivity : ComponentActivity() {
    private val actionScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    override fun onCreate(savedInstanceState: Bundle?) {
        showOverLockscreen()
        super.onCreate(savedInstanceState)

        val payload = AlarmPayload.from(intent)
        setContent {
            MaterialTheme {
                AlarmActivityContent(
                    payload = payload,
                    onAcknowledge = { runAlarmAction(payload, ReminderNotificationActionReceiver.ACTION_ACKNOWLEDGE) },
                    onSnooze = { runAlarmAction(payload, ReminderNotificationActionReceiver.ACTION_SNOOZE) },
                )
            }
        }
    }

    override fun onDestroy() {
        actionScope.cancel()
        super.onDestroy()
    }

    private fun showOverLockscreen() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O_MR1) {
            setShowWhenLocked(true)
            setTurnScreenOn(true)
        } else {
            @Suppress("DEPRECATION")
            window.addFlags(
                WindowManager.LayoutParams.FLAG_SHOW_WHEN_LOCKED or
                    WindowManager.LayoutParams.FLAG_TURN_SCREEN_ON,
            )
        }
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
    }

    private fun runAlarmAction(
        payload: AlarmPayload,
        action: String,
    ) {
        val actionIntent = Intent()
            .setAction(action)
            .putNotificationExtras(
                notificationId = payload.notificationId,
                itemId = payload.itemId,
                title = payload.title,
                message = payload.message,
                type = payload.type,
            )

        actionScope.launch {
            try {
                AttentionRuntimeCoordinator(this@AlarmActivity).handleNotificationIntent(actionIntent)
            } finally {
                runOnUiThread { finish() }
            }
        }
    }

}

private data class AlarmPayload(
    val notificationId: Int,
    val itemId: String,
    val title: String,
    val message: String,
    val type: String,
) {
    companion object {
        fun from(intent: Intent): AlarmPayload =
            AlarmPayload(
                notificationId = intent.getIntExtra(
                    ReminderNotificationService.EXTRA_NOTIFICATION_ID,
                    ReminderNotificationService.TEST_NOTIFICATION_ID,
                ),
                itemId = intent.getStringExtra(ReminderNotificationService.EXTRA_ITEM_ID)
                    ?: ReminderNotificationService.TEST_ITEM_ID,
                title = intent.getStringExtra(ReminderNotificationService.EXTRA_TITLE)
                    ?: ReminderNotificationService.TEST_TITLE,
                message = intent.getStringExtra(ReminderNotificationService.EXTRA_MESSAGE)
                    ?: ReminderNotificationService.TEST_MESSAGE,
                type = intent.getStringExtra(ReminderNotificationService.EXTRA_TYPE)
                    ?: AttentionType.Alarm.name,
            )
    }
}

@Composable
private fun AlarmActivityContent(
    payload: AlarmPayload,
    onAcknowledge: () -> Unit,
    onSnooze: () -> Unit,
) {
    var handling by remember { mutableStateOf(false) }

    Surface(
        modifier = Modifier.fillMaxSize(),
        color = MaterialTheme.colorScheme.errorContainer,
    ) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(24.dp),
            verticalArrangement = Arrangement.Center,
            horizontalAlignment = Alignment.Start,
        ) {
            Text(
                text = "Alarm",
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.onErrorContainer,
            )
            Spacer(Modifier.height(12.dp))
            Text(
                text = payload.title,
                style = MaterialTheme.typography.headlineMedium,
                fontWeight = FontWeight.Bold,
                color = MaterialTheme.colorScheme.onErrorContainer,
            )
            Spacer(Modifier.height(12.dp))
            Text(
                text = payload.message,
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onErrorContainer,
            )
            Spacer(Modifier.height(32.dp))
            Column(
                modifier = Modifier.fillMaxWidth(),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Button(
                    modifier = Modifier.fillMaxWidth(),
                    enabled = !handling,
                    onClick = {
                        handling = true
                        onSnooze()
                    },
                ) {
                    Text("Snooze 5 minutes")
                }
                Button(
                    modifier = Modifier.fillMaxWidth(),
                    enabled = !handling,
                    onClick = {
                        handling = true
                        onAcknowledge()
                    },
                ) {
                    Text("Acknowledge")
                }
            }
        }
    }
}
