package work.slhaf.agentic.console.platform.attention

import android.app.NotificationManager
import android.content.Context
import android.content.Intent
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.StateFlow
import work.slhaf.agentic.console.domain.attention.AttentionItem
import work.slhaf.agentic.console.domain.attention.AttentionRestoreAction
import work.slhaf.agentic.console.domain.attention.AttentionSourceKind
import work.slhaf.agentic.console.domain.attention.AttentionStatus
import work.slhaf.agentic.console.domain.attention.AttentionTransitionPolicy
import work.slhaf.agentic.console.domain.attention.AttentionType
import work.slhaf.agentic.console.domain.attention.ScheduleMode
import work.slhaf.agentic.console.platform.attention.persistence.AndroidRoomAttentionRepository
import work.slhaf.agentic.console.platform.attention.persistence.AttentionDatabase
import kotlin.time.Clock
import kotlin.time.Duration
import kotlin.time.Duration.Companion.minutes

interface AttentionTransitionOwner {
    fun observeItems(): StateFlow<List<AttentionItem>>

    suspend fun create(item: AttentionItem)
    suspend fun markDone(id: String)
    suspend fun acknowledge(id: String)
    suspend fun snooze(id: String, duration: Duration)
    suspend fun cancel(id: String)
    suspend fun clearMockData()
}

class AttentionRuntimeCoordinator(
    context: Context,
    scope: CoroutineScope,
) : AttentionTransitionOwner {
    private val appContext = context.applicationContext
    private val scheduler = AndroidAttentionScheduler(appContext)
    private val repository = AndroidRoomAttentionRepository(
        dao = AttentionDatabase.create(appContext).attentionDao(),
        scope = scope,
    )

    override fun observeItems(): StateFlow<List<AttentionItem>> = repository.observeItems()

    override suspend fun create(item: AttentionItem) {
        repository.create(item)
        val now = nowEpochMillis()
        when (AttentionTransitionPolicy.restoreAction(item, now)) {
            AttentionRestoreAction.Trigger -> triggerPersisted(item, now)
            AttentionRestoreAction.Schedule -> scheduleFuture(item)
            AttentionRestoreAction.Ignore -> Unit
        }
    }

    override suspend fun markDone(id: String) {
        markTerminal(id, AttentionStatus.Done)
    }

    override suspend fun acknowledge(id: String) {
        markTerminal(id, AttentionStatus.Acknowledged)
    }

    override suspend fun snooze(id: String, duration: Duration) {
        val item = repository.findById(id) ?: return
        val now = nowEpochMillis()
        val snoozed = AttentionTransitionPolicy.snoozed(item, duration, now) ?: return
        if (repository.snoozeIfActive(snoozed)) {
            val result = scheduler.snooze(snoozed)
            when (result.mode) {
                ScheduleMode.Overdue -> triggerPersisted(snoozed, nowEpochMillis())
                ScheduleMode.Failed -> repository.markFailedIfPending(snoozed)
                ScheduleMode.Degraded -> repository.markDegradedIfPending(snoozed)
                ScheduleMode.LocalAlarm -> repository.restoreWaitingIfDegraded(snoozed)
                else -> Unit
            }
            reconcilePendingSchedule(snoozed)
        }
    }

    override suspend fun cancel(id: String) {
        markTerminal(id, AttentionStatus.Cancelled)
    }

    override suspend fun clearMockData() {
        repository.queryBySourceKind(AttentionSourceKind.LocalMock)
            .forEach { scheduler.cancel(it.id) }
        repository.clearMockData()
    }

    suspend fun restorePendingItems() {
        val now = nowEpochMillis()
        repository.queryOverdueForRestore(now).forEach { item ->
            triggerPersisted(item, now)
        }
        repository.queryPendingForRestore(now).forEach { item ->
            scheduleFuture(item)
        }
    }

    suspend fun handleNotificationIntent(intent: Intent) {
        val payload = NotificationPayload.from(intent) ?: return
        when (intent.action) {
            ReminderNotificationActionReceiver.ACTION_DONE ->
                markTerminal(payload.itemId, AttentionStatus.Done, payload.notificationId)
            ReminderNotificationActionReceiver.ACTION_ACKNOWLEDGE ->
                markTerminal(payload.itemId, AttentionStatus.Acknowledged, payload.notificationId)
            ReminderNotificationActionReceiver.ACTION_SNOOZE -> snoozeNotification(payload)
            ReminderNotificationActionReceiver.ACTION_SHOW_SNOOZED_NOTIFICATION,
            ReminderNotificationActionReceiver.ACTION_FIRE_ATTENTION_ITEM -> fire(payload)
        }
    }

    private suspend fun snoozeNotification(payload: NotificationPayload) {
        val item = repository.findById(payload.itemId)
        if (item == null) {
            cancelNotification(payload.notificationId)
            return
        }
        cancelNotification(payload.notificationId)
        snooze(item.id, snoozeDelayMillis(item.type))
    }

    private suspend fun markTerminal(
        id: String,
        status: AttentionStatus,
        notificationId: Int? = null,
    ) {
        val transitioned = repository.markTerminalIfActive(id, status, nowEpochMillis())
        val current = if (transitioned) null else repository.findById(id)
        val shouldCancel = transitioned ||
            current == null ||
            current.status in AttentionTransitionPolicy.terminalStatuses
        if (shouldCancel) {
            scheduler.cancel(id)
            if (notificationId != null) cancelNotification(notificationId)
        }
    }

    private suspend fun fire(payload: NotificationPayload) {
        val item = repository.findById(payload.itemId)
        if (item == null) {
            // The Room fact was deleted; do not resurrect an unowned notification.
            cancelNotification(payload.notificationId)
            return
        }

        val now = nowEpochMillis()
        when (AttentionTransitionPolicy.restoreAction(item, now)) {
            AttentionRestoreAction.Trigger -> triggerPersisted(item, now)
            AttentionRestoreAction.Schedule -> scheduleFuture(item)
            AttentionRestoreAction.Ignore -> {
                if (item.status in AttentionTransitionPolicy.terminalStatuses) {
                    cancelNotification(payload.notificationId)
                }
            }
        }
    }

    private suspend fun triggerPersisted(item: AttentionItem, nowEpochMillis: Long) {
        val triggered = AttentionTransitionPolicy.triggered(item, nowEpochMillis) ?: return
        if (!repository.claimTriggered(item.id, nowEpochMillis)) return

        val result = scheduler.showNotification(triggered)
        if (!result.accepted) {
            repository.markFailedIfTriggered(item.id, nowEpochMillis())
        } else {
            reconcileTriggeredNotification(triggered)
        }
    }

    private suspend fun scheduleFuture(item: AttentionItem) {
        val now = nowEpochMillis()
        if (item.dueAtEpochMillis <= now) {
            triggerPersisted(item, now)
            return
        }
        val result = scheduler.schedule(item)
        when (result.mode) {
            ScheduleMode.Overdue -> triggerPersisted(item, nowEpochMillis())
            ScheduleMode.Failed -> repository.markFailedIfPending(item)
            ScheduleMode.Degraded -> repository.markDegradedIfPending(item)
            ScheduleMode.LocalAlarm -> repository.restoreWaitingIfDegraded(item)
            else -> Unit
        }
        reconcilePendingSchedule(item)
    }

    private suspend fun reconcilePendingSchedule(item: AttentionItem) {
        val current = repository.findById(item.id)
        when {
            current == null -> {
                scheduler.cancel(item.id)
                return
            }
            current.status == AttentionStatus.Triggered -> return
            current.status !in AttentionTransitionPolicy.pendingStatuses -> {
                scheduler.cancel(item.id)
                return
            }
        }

        val now = nowEpochMillis()
        if (current.dueAtEpochMillis <= now) {
            triggerPersisted(current, now)
        } else if (
            current.dueAtEpochMillis != item.dueAtEpochMillis ||
            current.updatedAtEpochMillis != item.updatedAtEpochMillis
        ) {
            reschedulePending(current)
        }
    }

    private suspend fun reconcileTriggeredNotification(item: AttentionItem) {
        val current = repository.findById(item.id)
        when {
            current?.status == AttentionStatus.Triggered -> Unit
            current != null && current.status in AttentionTransitionPolicy.pendingStatuses ->
                reschedulePending(current)
            else -> scheduler.cancel(item.id)
        }
    }

    private suspend fun reschedulePending(item: AttentionItem) {
        val result = scheduler.snooze(item)
        when (result.mode) {
            ScheduleMode.Overdue -> triggerPersisted(item, nowEpochMillis())
            ScheduleMode.Failed -> repository.markFailedIfPending(item)
            ScheduleMode.Degraded -> repository.markDegradedIfPending(item)
            ScheduleMode.LocalAlarm -> repository.restoreWaitingIfDegraded(item)
            else -> Unit
        }
        val latest = repository.findById(item.id)
        if (latest == null || latest.status in AttentionTransitionPolicy.terminalStatuses) {
            scheduler.cancel(item.id)
        }
    }

    private fun snoozeDelayMillis(type: AttentionType): Duration =
        when (type) {
            AttentionType.Reminder -> REMINDER_SNOOZE_DELAY_MILLIS
            AttentionType.Alarm -> ALARM_SNOOZE_DELAY_MILLIS
        }

    private fun cancelNotification(notificationId: Int) {
        appContext.getSystemService(NotificationManager::class.java).cancel(notificationId)
    }

    private fun nowEpochMillis(): Long = Clock.System.now().toEpochMilliseconds()

    private data class NotificationPayload(
        val notificationId: Int,
        val itemId: String,
    ) {
        companion object {
            fun from(intent: Intent): NotificationPayload? {
                val itemId = intent.getStringExtra(ReminderNotificationService.EXTRA_ITEM_ID)
                    ?: return null
                return NotificationPayload(
                    notificationId = intent.getIntExtra(
                        ReminderNotificationService.EXTRA_NOTIFICATION_ID,
                        itemId.hashCode(),
                    ),
                    itemId = itemId,
                )
            }
        }
    }

    private companion object {
        val REMINDER_SNOOZE_DELAY_MILLIS = 10.minutes
        val ALARM_SNOOZE_DELAY_MILLIS = 5.minutes
    }
}

