package work.slhaf.agentic.console.domain.attention

import kotlin.time.Duration.Companion.minutes

object AttentionDemoData {
    fun initialItems(nowEpochMillis: Long = currentEpochMillis()): List<AttentionItem> {
        fun atOffset(minutes: Long) = nowEpochMillis + minutes * 60_000

        return listOf(
            AttentionItem(
                id = "mock-reminder-hr",
                type = AttentionType.Reminder,
                title = "跟进 HR",
                message = "确认面试反馈和下一步安排",
                dueAtEpochMillis = atOffset(90),
                status = AttentionStatus.Waiting,
                source = AttentionSource(AttentionSourceKind.LocalMock, "本地 mock"),
                actions = listOf(AttentionAction.Done, AttentionAction.Snooze, AttentionAction.Cancel),
                scheduleKind = ScheduleKind.Flexible,
                reminderMetadata = ReminderMetadata(defaultSnooze = 15.minutes),
                createdAtEpochMillis = atOffset(-120),
                updatedAtEpochMillis = atOffset(-30),
            ),
            AttentionItem(
                id = "mock-alarm-exam",
                type = AttentionType.Alarm,
                title = "该出发考试了",
                message = "带证件、准考证和水",
                dueAtEpochMillis = atOffset(-3),
                status = AttentionStatus.Triggered,
                source = AttentionSource(AttentionSourceKind.LocalMock, "本地 mock"),
                actions = listOf(AttentionAction.Acknowledge, AttentionAction.Snooze),
                scheduleKind = ScheduleKind.ExactPreferred,
                alarmMetadata = AlarmMetadata(requiresAcknowledgement = true, defaultSnooze = 5.minutes),
                createdAtEpochMillis = atOffset(-180),
                updatedAtEpochMillis = atOffset(-3),
            ),
            AttentionItem(
                id = "mock-reminder-ci",
                type = AttentionType.Reminder,
                title = "看一下 CI",
                message = "检查 Android assemble 结果",
                dueAtEpochMillis = atOffset(30),
                status = AttentionStatus.Snoozed,
                source = AttentionSource(AttentionSourceKind.LocalMock, "本地 mock"),
                actions = listOf(AttentionAction.Done, AttentionAction.Snooze),
                scheduleKind = ScheduleKind.Flexible,
                reminderMetadata = ReminderMetadata(defaultSnooze = 10.minutes),
                createdAtEpochMillis = atOffset(-45),
                updatedAtEpochMillis = atOffset(-5),
            ),
            AttentionItem(
                id = "mock-alarm-wash",
                type = AttentionType.Alarm,
                title = "熄灯前洗漱",
                message = null,
                dueAtEpochMillis = atOffset(240),
                status = AttentionStatus.Waiting,
                source = AttentionSource(AttentionSourceKind.LocalMock, "本地 mock"),
                actions = listOf(AttentionAction.Acknowledge, AttentionAction.Snooze, AttentionAction.Cancel),
                scheduleKind = ScheduleKind.ExactPreferred,
                alarmMetadata = AlarmMetadata(requiresAcknowledgement = true, defaultSnooze = 5.minutes),
                createdAtEpochMillis = atOffset(-20),
                updatedAtEpochMillis = atOffset(-20),
            ),
            AttentionItem(
                id = "mock-reminder-bath",
                type = AttentionType.Reminder,
                title = "洗澡",
                message = "已在 mock 流程中标记完成",
                dueAtEpochMillis = atOffset(-60),
                status = AttentionStatus.Done,
                source = AttentionSource(AttentionSourceKind.LocalMock, "本地 mock"),
                actions = emptyList(),
                scheduleKind = ScheduleKind.Flexible,
                reminderMetadata = ReminderMetadata(defaultSnooze = 15.minutes),
                createdAtEpochMillis = atOffset(-180),
                updatedAtEpochMillis = atOffset(-55),
            ),
        )
    }

    fun createMockReminder(afterMinutes: Long = 1, nowEpochMillis: Long = currentEpochMillis()): AttentionItem {
        val dueAt = nowEpochMillis + afterMinutes * 60_000
        return AttentionItem(
            id = "mock-reminder-$nowEpochMillis",
            type = AttentionType.Reminder,
            title = "Mock Reminder",
            message = "仅创建本地 mock 数据，不会弹出系统通知",
            dueAtEpochMillis = dueAt,
            status = AttentionStatus.Waiting,
            source = AttentionSource(AttentionSourceKind.LocalMock, "本地 mock"),
            actions = listOf(AttentionAction.Done, AttentionAction.Snooze, AttentionAction.Cancel),
            scheduleKind = ScheduleKind.Flexible,
            reminderMetadata = ReminderMetadata(defaultSnooze = 15.minutes),
            createdAtEpochMillis = nowEpochMillis,
            updatedAtEpochMillis = nowEpochMillis,
        )
    }

    fun createMockAlarm(afterMinutes: Long = 1, nowEpochMillis: Long = currentEpochMillis()): AttentionItem {
        val dueAt = nowEpochMillis + afterMinutes * 60_000
        return AttentionItem(
            id = "mock-alarm-$nowEpochMillis",
            type = AttentionType.Alarm,
            title = "Mock Alarm",
            message = "仅创建本地 mock 数据，不会调用 AlarmManager",
            dueAtEpochMillis = dueAt,
            status = AttentionStatus.Waiting,
            source = AttentionSource(AttentionSourceKind.LocalMock, "本地 mock"),
            actions = listOf(AttentionAction.Acknowledge, AttentionAction.Snooze, AttentionAction.Cancel),
            scheduleKind = ScheduleKind.ExactPreferred,
            alarmMetadata = AlarmMetadata(requiresAcknowledgement = true, defaultSnooze = 5.minutes),
            createdAtEpochMillis = nowEpochMillis,
            updatedAtEpochMillis = nowEpochMillis,
        )
    }

    private fun currentEpochMillis(): Long = kotlin.time.Clock.System.now().toEpochMilliseconds()
}
