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
import work.slhaf.agentic.console.domain.attention.AttentionTransitionPolicy
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
        markTerminalIfActive(id, AttentionStatus.Done, nowEpochMillis())
    }

    override suspend fun acknowledge(id: String) {
        markTerminalIfActive(id, AttentionStatus.Acknowledged, nowEpochMillis())
    }

    override suspend fun snooze(id: String, duration: Duration) {
        val item = findById(id) ?: return
        val now = nowEpochMillis()
        val snoozed = AttentionTransitionPolicy.snoozed(item, duration, now) ?: return
        snoozeIfActive(snoozed)
    }

    override suspend fun cancel(id: String) {
        markTerminalIfActive(id, AttentionStatus.Cancelled, nowEpochMillis())
    }

    override suspend fun clearMockData() {
        dao.clearBySourceKind(AttentionSourceKind.LocalMock.name)
    }

    suspend fun findById(id: String): AttentionItem? =
        dao.findById(id)?.toDomain()

    suspend fun queryPendingForRestore(nowEpochMillis: Long): List<AttentionItem> =
        dao.queryPendingForRestore(
            statuses = AttentionTransitionPolicy.pendingStatuses.map { it.name },
            nowEpochMillis = nowEpochMillis,
        ).map { it.toDomain() }

    suspend fun queryOverdueForRestore(nowEpochMillis: Long): List<AttentionItem> =
        dao.queryOverdueForRestore(
            statuses = AttentionTransitionPolicy.pendingStatuses.map { it.name },
            nowEpochMillis = nowEpochMillis,
        ).map { it.toDomain() }

    suspend fun queryBySourceKind(sourceKind: AttentionSourceKind): List<AttentionItem> =
        dao.queryBySourceKind(sourceKind.name).map { it.toDomain() }

    suspend fun claimTriggered(id: String, nowEpochMillis: Long): Boolean =
        dao.claimTriggered(
            id = id,
            status = AttentionStatus.Triggered.name,
            pendingStatuses = AttentionTransitionPolicy.pendingStatuses.map { it.name },
            nowEpochMillis = nowEpochMillis,
            updatedAtEpochMillis = nowEpochMillis,
        ) > 0

    suspend fun markTerminalIfActive(
        id: String,
        status: AttentionStatus,
        nowEpochMillis: Long,
    ): Boolean {
        val item = findById(id) ?: return false
        if (AttentionTransitionPolicy.terminal(item, status, nowEpochMillis) == null) return false
        return dao.updateTerminalStateIfActive(
            id = id,
            status = status.name,
            activeStatuses = AttentionTransitionPolicy.actionableStatuses.map { it.name },
            updatedAtEpochMillis = nowEpochMillis,
        ) > 0
    }

    suspend fun markFailedIfTriggered(id: String, nowEpochMillis: Long): Boolean =
        dao.updateTerminalStateIfActive(
            id = id,
            status = AttentionStatus.Failed.name,
            activeStatuses = listOf(AttentionStatus.Triggered.name),
            updatedAtEpochMillis = nowEpochMillis,
        ) > 0

    suspend fun markFailedIfPending(item: AttentionItem): Boolean =
        dao.markFailedIfPending(
            id = item.id,
            failedStatus = AttentionStatus.Failed.name,
            pendingStatuses = AttentionTransitionPolicy.pendingStatuses.map { it.name },
            dueAtEpochMillis = item.dueAtEpochMillis,
            expectedUpdatedAtEpochMillis = item.updatedAtEpochMillis,
            updatedAtEpochMillis = nowEpochMillis(),
        ) > 0

    suspend fun markDegradedIfPending(item: AttentionItem): Boolean =
        dao.markDegradedIfPending(
            id = item.id,
            degradedStatus = AttentionStatus.Degraded.name,
            normalPendingStatuses = AttentionTransitionPolicy.normalPendingStatuses.map { it.name },
            dueAtEpochMillis = item.dueAtEpochMillis,
            expectedUpdatedAtEpochMillis = item.updatedAtEpochMillis,
            updatedAtEpochMillis = nowEpochMillis(),
        ) > 0

    /**
     * The v1 Room schema has no prior-status column; exact recovery therefore
     * canonicalizes a Degraded row back to Waiting while retaining its due time.
     */
    suspend fun restoreWaitingIfDegraded(item: AttentionItem): Boolean =
        dao.restoreWaitingIfDegraded(
            id = item.id,
            waitingStatus = AttentionStatus.Waiting.name,
            degradedStatus = AttentionStatus.Degraded.name,
            dueAtEpochMillis = item.dueAtEpochMillis,
            expectedUpdatedAtEpochMillis = item.updatedAtEpochMillis,
            updatedAtEpochMillis = nowEpochMillis(),
        ) > 0

    suspend fun snoozeIfActive(item: AttentionItem): Boolean =
        dao.snoozeIfActive(
            id = item.id,
            status = AttentionStatus.Snoozed.name,
            dueAtEpochMillis = item.dueAtEpochMillis,
            activeStatuses = AttentionTransitionPolicy.actionableStatuses.map { it.name },
            updatedAtEpochMillis = item.updatedAtEpochMillis,
        ) > 0

    private fun nowEpochMillis(): Long = Clock.System.now().toEpochMilliseconds()
}
