package work.slhaf.agentic.console.platform.attention

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import work.slhaf.agentic.console.domain.attention.AttentionDemoData
import work.slhaf.agentic.console.domain.attention.AttentionItem
import work.slhaf.agentic.console.domain.attention.AttentionRepository
import work.slhaf.agentic.console.domain.attention.AttentionStatus
import kotlin.time.Duration

class InMemoryAttentionRepository(
    initialItems: List<AttentionItem> = AttentionDemoData.initialItems(),
) : AttentionRepository {
    private val items = MutableStateFlow(initialItems)

    override fun observeItems(): StateFlow<List<AttentionItem>> = items

    override suspend fun create(item: AttentionItem) {
        items.update { current -> (current + item).sortedBy { it.dueAtEpochMillis } }
    }

    override suspend fun markDone(id: String) {
        updateItem(id) { it.copy(status = AttentionStatus.Done, actions = emptyList(), updatedAtEpochMillis = nowEpochMillis()) }
    }

    override suspend fun acknowledge(id: String) {
        updateItem(id) { it.copy(status = AttentionStatus.Acknowledged, actions = emptyList(), updatedAtEpochMillis = nowEpochMillis()) }
    }

    override suspend fun snooze(id: String, duration: Duration) {
        val current = nowEpochMillis()
        updateItem(id) {
            it.copy(
                status = AttentionStatus.Snoozed,
                dueAtEpochMillis = current + duration.inWholeMilliseconds,
                updatedAtEpochMillis = current,
            )
        }
    }

    override suspend fun cancel(id: String) {
        updateItem(id) { it.copy(status = AttentionStatus.Cancelled, actions = emptyList(), updatedAtEpochMillis = nowEpochMillis()) }
    }

    override suspend fun clearMockData() {
        items.value = emptyList()
    }

    private fun updateItem(id: String, transform: (AttentionItem) -> AttentionItem) {
        items.update { current ->
            current.map { item -> if (item.id == id) transform(item) else item }
                .sortedBy { it.dueAtEpochMillis }
        }
    }

    private fun nowEpochMillis(): Long = kotlin.time.Clock.System.now().toEpochMilliseconds()
}
