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
import kotlin.time.Duration

class AndroidAttentionScheduler(
    context: Context,
) : AttentionScheduler {
    private val appContext = context.applicationContext
    private val alarmManager = appContext.getSystemService(AlarmManager::class.java)
    private val notificationService = ReminderNotificationService(appContext)

    override fun schedule(item: AttentionItem): ScheduleResult {
        val now = System.currentTimeMillis()
        if (item.dueAtEpochMillis <= now) {
            val shown = notificationService.showAttentionNotification(item)
            return ScheduleResult(
                accepted = shown,
                mode = if (shown) ScheduleMode.LocalNotification else ScheduleMode.Failed,
                reason = if (shown) null else "Notification permission is not granted.",
            )
        }

        val pendingIntent = alarmPendingIntent(item, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        return when (item.scheduleKind) {
            ScheduleKind.Flexible -> {
                alarmManager.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, item.dueAtEpochMillis, pendingIntent)
                ScheduleResult(accepted = true, mode = ScheduleMode.LocalAlarm)
            }
            ScheduleKind.ExactPreferred,
            ScheduleKind.ExactRequired -> scheduleExactPreferred(item, pendingIntent)
        }
    }

    override fun cancel(itemId: String): ScheduleResult {
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
        return ScheduleResult(accepted = true, mode = ScheduleMode.LocalAlarm)
    }

    override fun snooze(itemId: String, duration: Duration): ScheduleResult =
        ScheduleResult(
            accepted = false,
            mode = ScheduleMode.Degraded,
            reason = "AndroidAttentionScheduler.snooze needs the full item payload; notification action snooze is handled by receiver extras in this spike.",
        )

    private fun scheduleExactPreferred(
        item: AttentionItem,
        pendingIntent: PendingIntent,
    ): ScheduleResult {
        if (canScheduleExactAlarms()) {
            try {
                alarmManager.setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, item.dueAtEpochMillis, pendingIntent)
                return ScheduleResult(accepted = true, mode = ScheduleMode.LocalAlarm)
            } catch (_: SecurityException) {
                alarmManager.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, item.dueAtEpochMillis, pendingIntent)
                return ScheduleResult(
                    accepted = true,
                    mode = ScheduleMode.Degraded,
                    reason = "Exact alarm access failed at schedule time; scheduled with inexact AlarmManager and may be delayed.",
                )
            }
        }

        alarmManager.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, item.dueAtEpochMillis, pendingIntent)
        return ScheduleResult(
            accepted = true,
            mode = ScheduleMode.Degraded,
            reason = "Exact alarm access is unavailable; scheduled with inexact AlarmManager and may be delayed.",
        )
    }

    private fun canScheduleExactAlarms(): Boolean =
        Build.VERSION.SDK_INT < Build.VERSION_CODES.S || alarmManager.canScheduleExactAlarms()

    private fun alarmPendingIntent(item: AttentionItem, flags: Int): PendingIntent =
        alarmPendingIntent(
            itemId = item.id,
            notificationId = item.notificationId(),
            title = item.title,
            message = item.notificationMessage(),
            type = item.type.name,
            flags = flags,
        ) ?: error("Unable to create alarm PendingIntent")

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
