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
        val title = intent.getStringExtra(ReminderNotificationService.EXTRA_TITLE)
            ?: ReminderNotificationService.TEST_TITLE
        val message = intent.getStringExtra(ReminderNotificationService.EXTRA_MESSAGE)
            ?: ReminderNotificationService.TEST_MESSAGE

        when (intent.action) {
            ACTION_DONE -> cancelNotification(context, notificationId)
            ACTION_SNOOZE_10_MINUTES -> {
                cancelNotification(context, notificationId)
                scheduleSnoozedNotification(context, notificationId, title, message)
            }
            ACTION_SHOW_SNOOZED_TEST_NOTIFICATION -> {
                ReminderNotificationService(context).showTestNotification(
                    notificationId = notificationId,
                    title = title,
                    message = message,
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
        title: String,
        message: String,
    ) {
        val appContext = context.applicationContext
        val intent = Intent(appContext, ReminderNotificationActionReceiver::class.java)
            .setAction(ACTION_SHOW_SNOOZED_TEST_NOTIFICATION)
            .putNotificationExtras(notificationId = notificationId, title = title, message = message)
        val pendingIntent = PendingIntent.getBroadcast(
            appContext,
            REQUEST_SHOW_SNOOZED_TEST_NOTIFICATION,
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
        const val ACTION_SHOW_SNOOZED_TEST_NOTIFICATION = "work.slhaf.agentic.console.action.SHOW_SNOOZED_TEST_NOTIFICATION"

        private const val REQUEST_SHOW_SNOOZED_TEST_NOTIFICATION = 2_003
        private const val SNOOZE_DELAY_MILLIS = 10 * 60 * 1_000L
    }
}
