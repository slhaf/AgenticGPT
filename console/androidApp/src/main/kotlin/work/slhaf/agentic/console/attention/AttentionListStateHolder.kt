package work.slhaf.agentic.console.attention

import androidx.compose.runtime.Immutable
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import work.slhaf.agentic.console.domain.attention.AttentionDemoData
import work.slhaf.agentic.console.domain.attention.AttentionItem
import work.slhaf.agentic.console.domain.attention.AttentionRepository
import work.slhaf.agentic.console.domain.attention.AttentionScheduler
import work.slhaf.agentic.console.domain.attention.AttentionStatus
import work.slhaf.agentic.console.domain.attention.AttentionType
import kotlin.time.Duration.Companion.minutes

class AttentionListStateHolder(
    private val repository: AttentionRepository,
    private val scheduler: AttentionScheduler,
    private val scope: CoroutineScope,
) {
    val state: StateFlow<AttentionListUiState> = repository.observeItems()
        .map { AttentionListUiState.from(it) }
        .stateIn(scope, SharingStarted.WhileSubscribed(5_000), AttentionListUiState())

    fun markDone(id: String) {
        scope.launch { repository.markDone(id) }
    }

    fun acknowledge(id: String) {
        scope.launch { repository.acknowledge(id) }
    }

    fun snooze(id: String) {
        scheduler.snooze(id, 5.minutes)
        scope.launch { repository.snooze(id, 5.minutes) }
    }

    fun cancel(id: String) {
        scheduler.cancel(id)
        scope.launch { repository.cancel(id) }
    }

    fun createMockReminder() {
        val item = AttentionDemoData.createMockReminder(afterMinutes = 1)
        scheduler.schedule(item)
        scope.launch { repository.create(item) }
    }

    fun createMockAlarm() {
        val item = AttentionDemoData.createMockAlarm(afterMinutes = 1)
        scheduler.schedule(item)
        scope.launch { repository.create(item) }
    }

    fun clearMockData() {
        scope.launch { repository.clearMockData() }
    }
}

@Immutable
data class AttentionListUiState(
    val items: List<AttentionItem> = emptyList(),
) {
    val waitingCount: Int = items.count { it.status == AttentionStatus.Waiting || it.status == AttentionStatus.Snoozed }
    val triggeredCount: Int = items.count { it.status == AttentionStatus.Triggered }
    val endedCount: Int = items.count { it.status in terminalStatuses }
    val nextItem: AttentionItem? = items
        .filter { it.status == AttentionStatus.Waiting || it.status == AttentionStatus.Snoozed }
        .minByOrNull { it.dueAtEpochMillis }

    fun filtered(filter: AttentionFilter): List<AttentionItem> =
        when (filter) {
            AttentionFilter.All -> items
            AttentionFilter.Reminder -> items.filter { it.type == AttentionType.Reminder }
            AttentionFilter.Alarm -> items.filter { it.type == AttentionType.Alarm }
        }

    companion object {
        val terminalStatuses = setOf(
            AttentionStatus.Done,
            AttentionStatus.Acknowledged,
            AttentionStatus.Cancelled,
            AttentionStatus.Failed,
        )

        fun from(items: List<AttentionItem>) = AttentionListUiState(items.sortedBy { it.dueAtEpochMillis })
    }
}

enum class AttentionFilter(val title: String) {
    All("全部"),
    Reminder("Reminder"),
    Alarm("Alarm"),
}
