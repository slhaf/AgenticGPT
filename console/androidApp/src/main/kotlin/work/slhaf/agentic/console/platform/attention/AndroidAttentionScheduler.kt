package work.slhaf.agentic.console.platform.attention

import android.app.AlarmManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Build
import work.slhaf.agentic.console.domain.attention.AttentionItem
import work.slhaf.agentic.console.domain.attention.AttentionScheduler
import work.slhaf.agentic.console.domain.attention.ScheduleKind
import work.slhaf.agentic.console.domain.attention.ScheduleMode
import work.slhaf.agentic.console.domain.attention.ScheduleResult

class AndroidAttentionScheduler(
    context: Context,
) : AttentionScheduler {
    private val appContext = context.applicationContext
    private val alarmManager = appContext.getSystemService(AlarmManager::class.java)
    private val notificationService = ReminderNotificationService(appContext)

    override fun schedule(item: AttentionItem): ScheduleResult =
        try {
            scheduleInternal(item)
        } catch (error: SecurityException) {
            ScheduleResult(
                accepted = false,
                mode = ScheduleMode.Failed,
                reason = error.message ?: "Android scheduling permission was denied.",
            )
        } catch (error: RuntimeException) {
            ScheduleResult(
                accepted = false,
                mode = ScheduleMode.Failed,
                reason = error.message ?: "Android alarm scheduling failed.",
            )
        }

    private fun scheduleInternal(item: AttentionItem): ScheduleResult {
        val now = System.currentTimeMillis()
        if (item.dueAtEpochMillis <= now) {
            return ScheduleResult(
                accepted = false,
                mode = ScheduleMode.Overdue,
                reason = "The deadline passed before AlarmManager scheduling; transition to Triggered before notification delivery.",
            )
        }

        val pendingIntent = alarmPendingIntent(
            item,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        ) ?: return ScheduleResult(
            accepted = false,
            mode = ScheduleMode.Failed,
            reason = "Unable to create an Android alarm PendingIntent.",
        )
        return when (item.scheduleKind) {
            ScheduleKind.Flexible -> {
                alarmManager.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, item.dueAtEpochMillis, pendingIntent)
                ScheduleResult(accepted = true, mode = ScheduleMode.LocalAlarm)
            }
            ScheduleKind.ExactPreferred,
            ScheduleKind.ExactRequired -> scheduleExactPreferred(item, pendingIntent)
        }
    }

    fun showNotification(item: AttentionItem): ScheduleResult =
        try {
            val shown = notificationService.showAttentionNotification(item)
            ScheduleResult(
                accepted = shown,
                mode = if (shown) ScheduleMode.LocalNotification else ScheduleMode.Failed,
                reason = if (shown) null else "Notification permission is not granted.",
            )
        } catch (error: SecurityException) {
            ScheduleResult(
                accepted = false,
                mode = ScheduleMode.Failed,
                reason = error.message ?: "Android notification permission was denied.",
            )
        } catch (error: RuntimeException) {
            ScheduleResult(
                accepted = false,
                mode = ScheduleMode.Failed,
                reason = error.message ?: "Android notification delivery failed.",
            )
        }

    override fun cancel(itemId: String): ScheduleResult =
        try {
            val pendingIntent = alarmPendingIntent(
                itemId = itemId,
                notificationId = itemId.hashCode(),
                title = "",
                message = "",
                type = null,
                flags = PendingIntent.FLAG_NO_CREATE or PendingIntent.FLAG_IMMUTABLE,
            )
            if (pendingIntent != null) {
                alarmManager.cancel(pendingIntent)
                pendingIntent.cancel()
            }
            val notificationManager = appContext.getSystemService(android.app.NotificationManager::class.java)
            notificationManager.cancel(itemId.hashCode())
            ScheduleResult(accepted = true, mode = ScheduleMode.LocalAlarm)
        } catch (error: SecurityException) {
            ScheduleResult(
                accepted = false,
                mode = ScheduleMode.Failed,
                reason = error.message ?: "Android scheduling permission was denied while cancelling.",
            )
        } catch (error: RuntimeException) {
            ScheduleResult(
                accepted = false,
                mode = ScheduleMode.Failed,
                reason = error.message ?: "Android schedule cancellation failed.",
            )
        }

    override fun snooze(item: AttentionItem): ScheduleResult {
        cancel(item.id)
        return schedule(item)
    }

    private fun scheduleExactPreferred(
        item: AttentionItem,
        pendingIntent: PendingIntent,
    ): ScheduleResult {
        if (canScheduleExactAlarms()) {
            try {
                alarmManager.setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, item.dueAtEpochMillis, pendingIntent)
                return ScheduleResult(accepted = true, mode = ScheduleMode.LocalAlarm)
            } catch (_: SecurityException) {
                // Fall through to the exact-required failure or preferred degraded path.
            }
        }
        if (item.scheduleKind == ScheduleKind.ExactRequired) {
            return ScheduleResult(
                accepted = false,
                mode = ScheduleMode.Failed,
                reason = "Exact alarm access is unavailable for an ExactRequired item.",
            )
        }

        return try {
            alarmManager.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, item.dueAtEpochMillis, pendingIntent)
            ScheduleResult(
                accepted = false,
                mode = ScheduleMode.Degraded,
                reason = "Exact alarm access is unavailable; an inexact AlarmManager alarm was requested and may be delayed.",
            )
        } catch (error: SecurityException) {
            ScheduleResult(
                accepted = false,
                mode = ScheduleMode.Failed,
                reason = error.message ?: "Android alarm scheduling permission was denied.",
            )
        }
    }

    private fun canScheduleExactAlarms(): Boolean =
        Build.VERSION.SDK_INT < Build.VERSION_CODES.S || alarmManager.canScheduleExactAlarms()

    private fun alarmPendingIntent(item: AttentionItem, flags: Int): PendingIntent? =
        alarmPendingIntent(
            itemId = item.id,
            notificationId = item.notificationId(),
            title = item.title,
            message = item.notificationMessage(),
            type = item.type.name,
            flags = flags,
        )

    private fun alarmPendingIntent(
        itemId: String,
        notificationId: Int,
        title: String,
        message: String,
        type: String?,
        flags: Int,
    ): PendingIntent? {
        val intent = Intent(appContext, ReminderNotificationActionReceiver::class.java)
            .setAction(ReminderNotificationActionReceiver.ACTION_FIRE_ATTENTION_ITEM)
            .putNotificationExtras(
                notificationId = notificationId,
                itemId = itemId,
                title = title,
                message = message,
                type = type,
            )
        return PendingIntent.getBroadcast(
            appContext,
            ReminderNotificationService.requestCode(itemId, ReminderNotificationActionReceiver.ACTION_FIRE_ATTENTION_ITEM),
            intent,
            flags,
        )
    }
}
