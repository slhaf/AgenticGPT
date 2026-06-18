package work.slhaf.agentic.console.platform.attention

import android.app.AlarmManager
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

class ReminderNotificationActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val notificationId = intent.getIntExtra(
            ReminderNotificationService.EXTRA_NOTIFICATION_ID,
            ReminderNotificationService.TEST_NOTIFICATION_ID,
        )
        val itemId = intent.getStringExtra(ReminderNotificationService.EXTRA_ITEM_ID)
            ?: ReminderNotificationService.TEST_ITEM_ID
        val title = intent.getStringExtra(ReminderNotificationService.EXTRA_TITLE)
            ?: ReminderNotificationService.TEST_TITLE
        val message = intent.getStringExtra(ReminderNotificationService.EXTRA_MESSAGE)
            ?: ReminderNotificationService.TEST_MESSAGE
        val type = intent.getStringExtra(ReminderNotificationService.EXTRA_TYPE)

        when (intent.action) {
            ACTION_DONE -> cancelNotification(context, notificationId)
            ACTION_SNOOZE_10_MINUTES -> {
                cancelNotification(context, notificationId)
                scheduleSnoozedNotification(context, notificationId, itemId, title, message, type)
            }
            ACTION_SHOW_SNOOZED_NOTIFICATION,
            ACTION_FIRE_ATTENTION_ITEM -> {
                ReminderNotificationService(context).showNotificationPayload(
                    notificationId = notificationId,
                    itemId = itemId,
                    title = title,
                    message = message,
                    type = type,
                )
            }
        }
    }

    private fun cancelNotification(context: Context, notificationId: Int) {
        val notificationManager = context.getSystemService(NotificationManager::class.java)
        notificationManager.cancel(notificationId)
    }

    private fun scheduleSnoozedNotification(
        context: Context,
        notificationId: Int,
        itemId: String,
        title: String,
        message: String,
        type: String?,
    ) {
        val appContext = context.applicationContext
        val intent = Intent(appContext, ReminderNotificationActionReceiver::class.java)
            .setAction(ACTION_SHOW_SNOOZED_NOTIFICATION)
            .putNotificationExtras(
                notificationId = notificationId,
                itemId = itemId,
                title = title,
                message = message,
                type = type,
            )
        val pendingIntent = PendingIntent.getBroadcast(
            appContext,
            ReminderNotificationService.requestCode(itemId, ACTION_SHOW_SNOOZED_NOTIFICATION),
            intent,
            ReminderNotificationService.pendingIntentFlags(),
        )
        val alarmManager = appContext.getSystemService(AlarmManager::class.java)
        alarmManager.setAndAllowWhileIdle(
            AlarmManager.RTC_WAKEUP,
            System.currentTimeMillis() + SNOOZE_DELAY_MILLIS,
            pendingIntent,
        )
    }

    companion object {
        const val ACTION_DONE = "work.slhaf.agentic.console.action.DONE"
        const val ACTION_SNOOZE_10_MINUTES = "work.slhaf.agentic.console.action.SNOOZE_10_MINUTES"
        const val ACTION_SHOW_SNOOZED_NOTIFICATION = "work.slhaf.agentic.console.action.SHOW_SNOOZED_NOTIFICATION"
        const val ACTION_FIRE_ATTENTION_ITEM = "work.slhaf.agentic.console.action.FIRE_ATTENTION_ITEM"

        private const val SNOOZE_DELAY_MILLIS = 10 * 60 * 1_000L
    }
}
