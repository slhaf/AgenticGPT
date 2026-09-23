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
import work.slhaf.agentic.console.domain.attention.AttentionStatus
import work.slhaf.agentic.console.domain.attention.AttentionType
import work.slhaf.agentic.console.platform.attention.AttentionTransitionOwner
import kotlin.time.Duration.Companion.minutes

class AttentionListStateHolder(
    private val owner: AttentionTransitionOwner,
    private val scope: CoroutineScope,
) {
    val state: StateFlow<AttentionListUiState> = owner.observeItems()
        .map { AttentionListUiState.from(it) }
        .stateIn(scope, SharingStarted.WhileSubscribed(5_000), AttentionListUiState())

    fun markDone(id: String) {
        scope.launch { owner.markDone(id) }
    }

    fun acknowledge(id: String) {
        scope.launch { owner.acknowledge(id) }
    }

    fun snooze(id: String) {
        scope.launch { owner.snooze(id, 5.minutes) }
    }

    fun cancel(id: String) {
        scope.launch { owner.cancel(id) }
    }

    fun createMockReminder() {
        scope.launch {
            owner.create(AttentionDemoData.createMockReminder(afterMinutes = 1))
        }
    }

    fun createMockAlarm() {
        scope.launch {
            owner.create(AttentionDemoData.createMockAlarm(afterMinutes = 1))
        }
    }

    fun clearMockData() {
        scope.launch { owner.clearMockData() }
    }
}

@Immutable
data class AttentionListUiState(
    val items: List<AttentionItem> = emptyList(),
) {
    val waitingCount: Int = items.count {
        it.status == AttentionStatus.Waiting ||
            it.status == AttentionStatus.Snoozed ||
            it.status == AttentionStatus.Degraded
    }
    val triggeredCount: Int = items.count { it.status == AttentionStatus.Triggered }
    val endedCount: Int = items.count { it.status in terminalStatuses }
    val nextItem: AttentionItem? = items
        .filter {
            it.status == AttentionStatus.Waiting ||
                it.status == AttentionStatus.Snoozed ||
                it.status == AttentionStatus.Degraded
        }
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
