package work.slhaf.agentic.console.domain.attention

interface AttentionScheduler {
    fun schedule(item: AttentionItem): ScheduleResult
    fun cancel(itemId: String): ScheduleResult
    fun snooze(item: AttentionItem): ScheduleResult
}

/**
 * [accepted] is true only when the requested local scheduling contract is met.
 * A degraded best-effort OS request is reported with [ScheduleMode.Degraded]
 * and must not be treated as success.
 */
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
    Overdue,
}
