package work.slhaf.agentic.console.domain.attention

import kotlin.time.Duration

data class AttentionItem(
    val id: String,
    val type: AttentionType,
    val title: String,
    val message: String?,
    val dueAtEpochMillis: Long,
    val status: AttentionStatus,
    val source: AttentionSource,
    val actions: List<AttentionAction>,
    val scheduleKind: ScheduleKind,
    val reminderMetadata: ReminderMetadata? = null,
    val alarmMetadata: AlarmMetadata? = null,
    val createdAtEpochMillis: Long,
    val updatedAtEpochMillis: Long,
)

enum class AttentionType {
    Reminder,
    Alarm,
}

enum class AttentionStatus {
    Waiting,
    Triggered,
    Snoozed,
    Done,
    Acknowledged,
    Cancelled,
    Failed,
}

enum class AttentionAction {
    Done,
    Snooze,
    Acknowledge,
    Cancel,
}

enum class ScheduleKind {
    Flexible,
    ExactPreferred,
    ExactRequired,
}

data class AttentionSource(
    val kind: AttentionSourceKind,
    val label: String,
)

enum class AttentionSourceKind {
    LocalMock,
    Hub,
    User,
}

data class ReminderMetadata(
    val defaultSnooze: Duration,
)

data class AlarmMetadata(
    val requiresAcknowledgement: Boolean,
    val defaultSnooze: Duration,
)

fun AttentionItem.isTerminal(): Boolean =
    status == AttentionStatus.Done ||
        status == AttentionStatus.Acknowledged ||
        status == AttentionStatus.Cancelled ||
        status == AttentionStatus.Failed
