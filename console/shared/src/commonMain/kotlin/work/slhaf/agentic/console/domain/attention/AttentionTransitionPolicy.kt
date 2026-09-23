package work.slhaf.agentic.console.domain.attention

import kotlin.time.Duration

/**
 * Pure local state transition rules shared by the Android UI and OS receivers.
 *
 * Persistence and notification/alarm effects stay outside this policy. Callers
 * must persist the returned item (or atomically claim the transition) before
 * applying an OS effect.
 */
object AttentionTransitionPolicy {
    val normalPendingStatuses: List<AttentionStatus> = listOf(
        AttentionStatus.Waiting,
        AttentionStatus.Snoozed,
    )

    val pendingStatuses: List<AttentionStatus> = normalPendingStatuses + AttentionStatus.Degraded

    val actionableStatuses: List<AttentionStatus> = listOf(
        AttentionStatus.Waiting,
        AttentionStatus.Snoozed,
        AttentionStatus.Degraded,
        AttentionStatus.Triggered,
    )

    val terminalStatuses: List<AttentionStatus> = listOf(
        AttentionStatus.Done,
        AttentionStatus.Acknowledged,
        AttentionStatus.Cancelled,
        AttentionStatus.Failed,
    )

    fun restoreAction(
        item: AttentionItem,
        nowEpochMillis: Long,
    ): AttentionRestoreAction =
        when {
            item.status !in pendingStatuses -> AttentionRestoreAction.Ignore
            item.dueAtEpochMillis <= nowEpochMillis -> AttentionRestoreAction.Trigger
            else -> AttentionRestoreAction.Schedule
        }

    fun triggered(
        item: AttentionItem,
        nowEpochMillis: Long,
    ): AttentionItem? =
        if (restoreAction(item, nowEpochMillis) == AttentionRestoreAction.Trigger) {
            item.copy(
                status = AttentionStatus.Triggered,
                updatedAtEpochMillis = nowEpochMillis,
            )
        } else {
            null
        }

    fun snoozed(
        item: AttentionItem,
        duration: Duration,
        nowEpochMillis: Long,
    ): AttentionItem? =
        if (item.status in actionableStatuses) {
            item.copy(
                status = AttentionStatus.Snoozed,
                dueAtEpochMillis = nowEpochMillis + duration.inWholeMilliseconds,
                updatedAtEpochMillis = nowEpochMillis,
            )
        } else {
            null
        }

    fun terminal(
        item: AttentionItem,
        status: AttentionStatus,
        nowEpochMillis: Long,
    ): AttentionItem? =
        if (status in terminalStatuses && item.status !in terminalStatuses) {
            item.copy(
                status = status,
                actions = emptyList(),
                updatedAtEpochMillis = nowEpochMillis,
            )
        } else {
            null
        }
}

enum class AttentionRestoreAction {
    Schedule,
    Trigger,
    Ignore,
}
