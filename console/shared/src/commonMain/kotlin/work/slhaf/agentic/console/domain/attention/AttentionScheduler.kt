package work.slhaf.agentic.console.domain.attention

import kotlin.time.Duration

interface AttentionScheduler {
    fun schedule(item: AttentionItem): ScheduleResult
    fun cancel(itemId: String): ScheduleResult
    fun snooze(itemId: String, duration: Duration): ScheduleResult
}

data class ScheduleResult(
    val accepted: Boolean,
    val mode: ScheduleMode,
    val reason: String? = null,
)

enum class ScheduleMode {
    MockOnly,
    LocalNotification,
    LocalAlarm,
    Degraded,
    Failed,
}
