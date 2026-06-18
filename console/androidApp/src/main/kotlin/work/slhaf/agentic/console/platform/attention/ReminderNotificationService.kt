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
import work.slhaf.agentic.console.domain.attention.AttentionItem
import work.slhaf.agentic.console.domain.attention.AttentionType
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
        itemId: String = TEST_ITEM_ID,
        title: String = TEST_TITLE,
        message: String = TEST_MESSAGE,
    ): Boolean {
        ensureChannel()
        if (!canPostNotifications()) return false

        showNotification(
            notificationId = notificationId,
            itemId = itemId,
            title = title,
            message = message,
        )
        return true
    }

    fun showAttentionNotification(item: AttentionItem): Boolean =
        showNotificationPayload(
            notificationId = item.notificationId(),
            itemId = item.id,
            title = item.title,
            message = item.notificationMessage(),
            type = item.type.name,
        )

    fun showNotificationPayload(
        notificationId: Int,
        itemId: String,
        title: String,
        message: String,
        type: String? = null,
    ): Boolean {
        ensureChannel()
        if (!canPostNotifications()) return false

        showNotification(
            notificationId = notificationId,
            itemId = itemId,
            title = title,
            message = message,
            type = type,
        )
        return true
    }

    @SuppressLint("MissingPermission")
    @Suppress("DEPRECATION")
    private fun showNotification(
        notificationId: Int,
        itemId: String,
        title: String,
        message: String,
        type: String? = null,
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
                        requestCode = requestCode(itemId, ReminderNotificationActionReceiver.ACTION_DONE),
                        notificationId = notificationId,
                        itemId = itemId,
                        title = title,
                        message = message,
                        type = type,
                    ),
                ).build(),
            )
            .addAction(
                Notification.Action.Builder(
                    R.drawable.ic_stat_agentic_notification,
                    "Snooze 10 minutes",
                    actionPendingIntent(
                        action = ReminderNotificationActionReceiver.ACTION_SNOOZE_10_MINUTES,
                        requestCode = requestCode(itemId, ReminderNotificationActionReceiver.ACTION_SNOOZE_10_MINUTES),
                        notificationId = notificationId,
                        itemId = itemId,
                        title = title,
                        message = message,
                        type = type,
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
        itemId: String,
        title: String,
        message: String,
        type: String?,
    ): PendingIntent {
        val intent = Intent(appContext, ReminderNotificationActionReceiver::class.java)
            .setAction(action)
            .putNotificationExtras(
                notificationId = notificationId,
                itemId = itemId,
                title = title,
                message = message,
                type = type,
            )

        return PendingIntent.getBroadcast(appContext, requestCode, intent, pendingIntentFlags())
    }

    companion object {
        const val CHANNEL_ID = "agentic_reminders"
        const val TEST_NOTIFICATION_ID = 1_001
        const val TEST_ITEM_ID = "debug-test-notification"
        const val TEST_TITLE = "Agentic reminder"
        const val TEST_MESSAGE = "This is a local notification spike."

        const val EXTRA_NOTIFICATION_ID = "work.slhaf.agentic.console.extra.NOTIFICATION_ID"
        const val EXTRA_ITEM_ID = "work.slhaf.agentic.console.extra.ITEM_ID"
        const val EXTRA_TITLE = "work.slhaf.agentic.console.extra.TITLE"
        const val EXTRA_MESSAGE = "work.slhaf.agentic.console.extra.MESSAGE"
        const val EXTRA_TYPE = "work.slhaf.agentic.console.extra.TYPE"

        private const val REQUEST_OPEN_APP = 2_000

        fun pendingIntentFlags(): Int = PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE

        fun requestCode(itemId: String, action: String): Int = 31 * itemId.hashCode() + action.hashCode()
    }
}

fun AttentionItem.notificationId(): Int = id.hashCode()

fun AttentionItem.notificationMessage(): String =
    message ?: when (type) {
        AttentionType.Reminder -> "Reminder due now"
        AttentionType.Alarm -> "Alarm due now"
    }

fun Intent.putNotificationExtras(
    notificationId: Int,
    itemId: String,
    title: String,
    message: String,
    type: String? = null,
): Intent = putExtra(ReminderNotificationService.EXTRA_NOTIFICATION_ID, notificationId)
    .putExtra(ReminderNotificationService.EXTRA_ITEM_ID, itemId)
    .putExtra(ReminderNotificationService.EXTRA_TITLE, title)
    .putExtra(ReminderNotificationService.EXTRA_MESSAGE, message)
    .apply {
        if (type != null) {
            putExtra(ReminderNotificationService.EXTRA_TYPE, type)
        }
    }
