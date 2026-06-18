package work.slhaf.agentic.console.platform.attention

import android.app.NotificationManager
import android.content.Context
import android.content.Intent
import work.slhaf.agentic.console.domain.attention.AttentionItem
import work.slhaf.agentic.console.domain.attention.AttentionStatus
import work.slhaf.agentic.console.domain.attention.AttentionType
import work.slhaf.agentic.console.platform.attention.persistence.AttentionDatabase
import work.slhaf.agentic.console.platform.attention.persistence.toDomain
import kotlin.time.Clock

class AttentionRuntimeCoordinator(
    context: Context,
) {
    private val appContext = context.applicationContext
    private val dao = AttentionDatabase.create(appContext).attentionDao()
    private val scheduler = AndroidAttentionScheduler(appContext)
    private val notificationService = ReminderNotificationService(appContext)

    suspend fun restoreFutureItems() {
        val now = nowEpochMillis()
        dao.queryPendingForRestore(
            statuses = listOf(AttentionStatus.Waiting.name, AttentionStatus.Snoozed.name),
            nowEpochMillis = now,
        ).forEach { entity ->
            scheduler.schedule(entity.toDomain())
        }
    }

    suspend fun handleNotificationIntent(intent: Intent) {
        val payload = NotificationPayload.from(intent)
        when (intent.action) {
            ReminderNotificationActionReceiver.ACTION_DONE -> markTerminal(
                payload = payload,
                status = AttentionStatus.Done,
            )
            ReminderNotificationActionReceiver.ACTION_ACKNOWLEDGE -> markTerminal(
                payload = payload,
                status = AttentionStatus.Acknowledged,
            )
            ReminderNotificationActionReceiver.ACTION_SNOOZE -> snooze(payload)
            ReminderNotificationActionReceiver.ACTION_SHOW_SNOOZED_NOTIFICATION,
            ReminderNotificationActionReceiver.ACTION_FIRE_ATTENTION_ITEM -> fire(payload)
        }
    }

    private suspend fun fire(payload: NotificationPayload) {
        val item = dao.findById(payload.itemId)?.toDomain()
        if (item != null) {
            val now = nowEpochMillis()
            dao.markTriggered(
                id = item.id,
                status = AttentionStatus.Triggered.name,
                updatedAtEpochMillis = now,
            )
            notificationService.showAttentionNotification(
                item.copy(status = AttentionStatus.Triggered, updatedAtEpochMillis = now),
            )
        } else {
            notificationService.showNotificationPayload(
                notificationId = payload.notificationId,
                itemId = payload.itemId,
                title = payload.title,
                message = payload.message,
                type = payload.type,
            )
        }
    }

    private suspend fun markTerminal(
        payload: NotificationPayload,
        status: AttentionStatus,
    ) {
        cancelNotification(payload.notificationId)
        dao.updateTerminalState(
            id = payload.itemId,
            status = status.name,
            updatedAtEpochMillis = nowEpochMillis(),
        )
    }

    private suspend fun snooze(payload: NotificationPayload) {
        cancelNotification(payload.notificationId)

        val item = dao.findById(payload.itemId)?.toDomain()
        val type = item?.type ?: payload.type.toAttentionType()
        val now = nowEpochMillis()
        val dueAt = now + snoozeDelayMillis(type)
        if (item != null) {
            dao.snooze(
                id = item.id,
                status = AttentionStatus.Snoozed.name,
                dueAtEpochMillis = dueAt,
                updatedAtEpochMillis = now,
            )
            scheduler.schedule(
                item.copy(
                    status = AttentionStatus.Snoozed,
                    dueAtEpochMillis = dueAt,
                    updatedAtEpochMillis = now,
                ),
            )
        } else {
            scheduleFallbackSnoozedNotification(payload, dueAt)
        }
    }

    private fun scheduleFallbackSnoozedNotification(
        payload: NotificationPayload,
        dueAtEpochMillis: Long,
    ) {
        val intent = Intent(appContext, ReminderNotificationActionReceiver::class.java)
            .setAction(ReminderNotificationActionReceiver.ACTION_SHOW_SNOOZED_NOTIFICATION)
            .putNotificationExtras(
                notificationId = payload.notificationId,
                itemId = payload.itemId,
                title = payload.title,
                message = payload.message,
                type = payload.type,
            )
        val pendingIntent = android.app.PendingIntent.getBroadcast(
            appContext,
            ReminderNotificationService.requestCode(
                payload.itemId,
                ReminderNotificationActionReceiver.ACTION_SHOW_SNOOZED_NOTIFICATION,
            ),
            intent,
            ReminderNotificationService.pendingIntentFlags(),
        )
        val alarmManager = appContext.getSystemService(android.app.AlarmManager::class.java)
        alarmManager.setAndAllowWhileIdle(android.app.AlarmManager.RTC_WAKEUP, dueAtEpochMillis, pendingIntent)
    }

    private fun cancelNotification(notificationId: Int) {
        val notificationManager = appContext.getSystemService(NotificationManager::class.java)
        notificationManager.cancel(notificationId)
    }

    private fun snoozeDelayMillis(type: AttentionType): Long =
        when (type) {
            AttentionType.Reminder -> REMINDER_SNOOZE_DELAY_MILLIS
            AttentionType.Alarm -> ALARM_SNOOZE_DELAY_MILLIS
        }

    private fun String?.toAttentionType(): AttentionType =
        if (this == AttentionType.Alarm.name) AttentionType.Alarm else AttentionType.Reminder

    private fun nowEpochMillis(): Long = Clock.System.now().toEpochMilliseconds()

    private data class NotificationPayload(
        val notificationId: Int,
        val itemId: String,
        val title: String,
        val message: String,
        val type: String?,
    ) {
        companion object {
            fun from(intent: Intent): NotificationPayload =
                NotificationPayload(
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
                    type = intent.getStringExtra(ReminderNotificationService.EXTRA_TYPE),
                )
        }
    }

    private companion object {
        const val REMINDER_SNOOZE_DELAY_MILLIS = 10 * 60 * 1_000L
        const val ALARM_SNOOZE_DELAY_MILLIS = 5 * 60 * 1_000L
    }
}
