package work.slhaf.agentic.console.platform.attention

import work.slhaf.agentic.console.domain.attention.AttentionItem
import work.slhaf.agentic.console.domain.attention.AttentionScheduler
import work.slhaf.agentic.console.domain.attention.ScheduleMode
import work.slhaf.agentic.console.domain.attention.ScheduleResult
import kotlin.time.Duration

class MockAttentionScheduler : AttentionScheduler {
    override fun schedule(item: AttentionItem): ScheduleResult =
        ScheduleResult(
            accepted = true,
            mode = ScheduleMode.MockOnly,
            reason = "Mock only: no Android notification or AlarmManager schedule is created.",
        )

    override fun cancel(itemId: String): ScheduleResult =
        ScheduleResult(
            accepted = true,
            mode = ScheduleMode.MockOnly,
            reason = "Mock only: no Android system schedule is cancelled.",
        )

    override fun snooze(itemId: String, duration: Duration): ScheduleResult =
        ScheduleResult(
            accepted = true,
            mode = ScheduleMode.MockOnly,
            reason = "Mock only: snooze updates in-memory state only.",
        )
}
