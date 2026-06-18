package work.slhaf.agentic.console.platform.attention.persistence

import work.slhaf.agentic.console.domain.attention.AlarmMetadata
import work.slhaf.agentic.console.domain.attention.AttentionAction
import work.slhaf.agentic.console.domain.attention.AttentionItem
import work.slhaf.agentic.console.domain.attention.AttentionSource
import work.slhaf.agentic.console.domain.attention.AttentionSourceKind
import work.slhaf.agentic.console.domain.attention.AttentionStatus
import work.slhaf.agentic.console.domain.attention.AttentionType
import work.slhaf.agentic.console.domain.attention.ReminderMetadata
import work.slhaf.agentic.console.domain.attention.ScheduleKind
import kotlin.time.Duration.Companion.milliseconds

private const val ACTION_SEPARATOR = "|"

fun AttentionItem.toEntity(): AttentionEntity =
    AttentionEntity(
        id = id,
        type = type.name,
        title = title,
        message = message,
        dueAtEpochMillis = dueAtEpochMillis,
        status = status.name,
        sourceKind = source.kind.name,
        sourceLabel = source.label,
        actions = actions.joinToString(ACTION_SEPARATOR) { it.name },
        scheduleKind = scheduleKind.name,
        reminderDefaultSnoozeMillis = reminderMetadata?.defaultSnooze?.inWholeMilliseconds,
        alarmRequiresAcknowledgement = alarmMetadata?.requiresAcknowledgement,
        alarmDefaultSnoozeMillis = alarmMetadata?.defaultSnooze?.inWholeMilliseconds,
        createdAtEpochMillis = createdAtEpochMillis,
        updatedAtEpochMillis = updatedAtEpochMillis,
    )

fun AttentionEntity.toDomain(): AttentionItem =
    AttentionItem(
        id = id,
        type = enumValueOrDefault(type, AttentionType.Reminder),
        title = title,
        message = message,
        dueAtEpochMillis = dueAtEpochMillis,
        status = enumValueOrDefault(status, AttentionStatus.Waiting),
        source = AttentionSource(
            kind = enumValueOrDefault(sourceKind, AttentionSourceKind.LocalMock),
            label = sourceLabel,
        ),
        actions = actions.split(ACTION_SEPARATOR)
            .filter { it.isNotBlank() }
            .mapNotNull { enumValueOrNull<AttentionAction>(it) },
        scheduleKind = enumValueOrDefault(scheduleKind, ScheduleKind.Flexible),
        reminderMetadata = reminderDefaultSnoozeMillis?.let { ReminderMetadata(defaultSnooze = it.milliseconds) },
        alarmMetadata = alarmMetadata(),
        createdAtEpochMillis = createdAtEpochMillis,
        updatedAtEpochMillis = updatedAtEpochMillis,
    )

private fun AttentionEntity.alarmMetadata(): AlarmMetadata? {
    val requiresAcknowledgement = alarmRequiresAcknowledgement
    val defaultSnoozeMillis = alarmDefaultSnoozeMillis
    if (requiresAcknowledgement == null && defaultSnoozeMillis == null) return null

    return AlarmMetadata(
        requiresAcknowledgement = requiresAcknowledgement ?: false,
        defaultSnooze = (defaultSnoozeMillis ?: 0L).milliseconds,
    )
}

private inline fun <reified T : Enum<T>> enumValueOrDefault(value: String, default: T): T =
    enumValueOrNull(value) ?: default

private inline fun <reified T : Enum<T>> enumValueOrNull(value: String): T? =
    enumValues<T>().firstOrNull { it.name == value }
