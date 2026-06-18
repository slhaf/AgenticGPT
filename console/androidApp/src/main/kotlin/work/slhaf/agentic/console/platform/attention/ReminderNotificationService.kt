package work.slhaf.agentic.console.platform.attention

import android.Manifest
import android.annotation.SuppressLint
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import work.slhaf.agentic.console.MainActivity
import work.slhaf.agentic.console.R

class ReminderNotificationService(
    context: Context,
) {
    private val appContext = context.applicationContext

    fun ensureChannel() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return

        val channel = NotificationChannel(
            CHANNEL_ID,
            "Agentic reminders",
            NotificationManager.IMPORTANCE_DEFAULT,
        ).apply {
            description = "Local reminder notifications for Agentic Console"
        }
        val notificationManager = appContext.getSystemService(NotificationManager::class.java)
        notificationManager.createNotificationChannel(channel)
    }

    fun canPostNotifications(): Boolean =
        Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
            appContext.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED

    fun showTestNotification(
        notificationId: Int = TEST_NOTIFICATION_ID,
        title: String = TEST_TITLE,
        message: String = TEST_MESSAGE,
    ): Boolean {
        ensureChannel()
        if (!canPostNotifications()) return false

        showNotification(
            notificationId = notificationId,
            title = title,
            message = message,
        )
        return true
    }

    @SuppressLint("MissingPermission")
    @Suppress("DEPRECATION")
    private fun showNotification(
        notificationId: Int,
        title: String,
        message: String,
    ) {
        val contentIntent = PendingIntent.getActivity(
            appContext,
            REQUEST_OPEN_APP,
            Intent(appContext, MainActivity::class.java),
            pendingIntentFlags(),
        )

        val notificationBuilder = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Notification.Builder(appContext, CHANNEL_ID)
        } else {
            Notification.Builder(appContext)
        }

        val notification = notificationBuilder
            .setSmallIcon(R.drawable.ic_stat_agentic_notification)
            .setContentTitle(title)
            .setContentText(message)
            .setStyle(Notification.BigTextStyle().bigText(message))
            .setContentIntent(contentIntent)
            .setAutoCancel(true)
            .setPriority(Notification.PRIORITY_DEFAULT)
            .addAction(
                Notification.Action.Builder(
                    R.drawable.ic_stat_agentic_notification,
                    "Done",
                    actionPendingIntent(
                        action = ReminderNotificationActionReceiver.ACTION_DONE,
                        requestCode = REQUEST_DONE,
                        notificationId = notificationId,
                        title = title,
                        message = message,
                    ),
                ).build(),
            )
            .addAction(
                Notification.Action.Builder(
                    R.drawable.ic_stat_agentic_notification,
                    "Snooze 10 minutes",
                    actionPendingIntent(
                        action = ReminderNotificationActionReceiver.ACTION_SNOOZE_10_MINUTES,
                        requestCode = REQUEST_SNOOZE,
                        notificationId = notificationId,
                        title = title,
                        message = message,
                    ),
                ).build(),
            )
            .build()

        val notificationManager = appContext.getSystemService(NotificationManager::class.java)
        notificationManager.notify(notificationId, notification)
    }

    private fun actionPendingIntent(
        action: String,
        requestCode: Int,
        notificationId: Int,
        title: String,
        message: String,
    ): PendingIntent {
        val intent = Intent(appContext, ReminderNotificationActionReceiver::class.java)
            .setAction(action)
            .putNotificationExtras(notificationId = notificationId, title = title, message = message)

        return PendingIntent.getBroadcast(appContext, requestCode, intent, pendingIntentFlags())
    }

    companion object {
        const val CHANNEL_ID = "agentic_reminders"
        const val TEST_NOTIFICATION_ID = 1_001
        const val TEST_TITLE = "Agentic reminder"
        const val TEST_MESSAGE = "This is a local notification spike."

        const val EXTRA_NOTIFICATION_ID = "work.slhaf.agentic.console.extra.NOTIFICATION_ID"
        const val EXTRA_TITLE = "work.slhaf.agentic.console.extra.TITLE"
        const val EXTRA_MESSAGE = "work.slhaf.agentic.console.extra.MESSAGE"

        private const val REQUEST_OPEN_APP = 2_000
        private const val REQUEST_DONE = 2_001
        private const val REQUEST_SNOOZE = 2_002

        fun pendingIntentFlags(): Int = PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
    }
}

fun Intent.putNotificationExtras(
    notificationId: Int,
    title: String,
    message: String,
): Intent = putExtra(ReminderNotificationService.EXTRA_NOTIFICATION_ID, notificationId)
    .putExtra(ReminderNotificationService.EXTRA_TITLE, title)
    .putExtra(ReminderNotificationService.EXTRA_MESSAGE, message)
