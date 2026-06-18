package work.slhaf.agentic.console.platform.attention.persistence

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import work.slhaf.agentic.console.domain.attention.AttentionItem
import work.slhaf.agentic.console.domain.attention.AttentionRepository
import work.slhaf.agentic.console.domain.attention.AttentionSourceKind
import work.slhaf.agentic.console.domain.attention.AttentionStatus
import kotlin.time.Clock
import kotlin.time.Duration

class AndroidRoomAttentionRepository(
    private val dao: AttentionDao,
    scope: CoroutineScope,
) : AttentionRepository {
    private val items = dao.observeItems()
        .map { entities -> entities.map { it.toDomain() } }
        .stateIn(scope, SharingStarted.WhileSubscribed(5_000), emptyList())

    override fun observeItems(): StateFlow<List<AttentionItem>> = items

    override suspend fun create(item: AttentionItem) {
        dao.upsert(item.toEntity())
    }

    override suspend fun markDone(id: String) {
        dao.updateTerminalState(
            id = id,
            status = AttentionStatus.Done.name,
            updatedAtEpochMillis = nowEpochMillis(),
        )
    }

    override suspend fun acknowledge(id: String) {
        dao.updateTerminalState(
            id = id,
            status = AttentionStatus.Acknowledged.name,
            updatedAtEpochMillis = nowEpochMillis(),
        )
    }

    override suspend fun snooze(id: String, duration: Duration) {
        val now = nowEpochMillis()
        dao.snooze(
            id = id,
            status = AttentionStatus.Snoozed.name,
            dueAtEpochMillis = now + duration.inWholeMilliseconds,
            updatedAtEpochMillis = now,
        )
    }

    override suspend fun cancel(id: String) {
        dao.updateTerminalState(
            id = id,
            status = AttentionStatus.Cancelled.name,
            updatedAtEpochMillis = nowEpochMillis(),
        )
    }

    override suspend fun clearMockData() {
        dao.clearBySourceKind(AttentionSourceKind.LocalMock.name)
    }

    private fun nowEpochMillis(): Long = Clock.System.now().toEpochMilliseconds()
}
