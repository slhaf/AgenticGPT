package work.slhaf.agentic.console.platform.attention.persistence

import androidx.room.Dao
import androidx.room.Insert
import androidx.room.OnConflictStrategy
import androidx.room.Query
import androidx.room.Upsert
import kotlinx.coroutines.flow.Flow

@Dao
interface AttentionDao {
    @Query("SELECT * FROM attention_items ORDER BY dueAtEpochMillis ASC")
    fun observeItems(): Flow<List<AttentionEntity>>

    @Query("SELECT * FROM attention_items WHERE id = :id LIMIT 1")
    suspend fun findById(id: String): AttentionEntity?

    @Query(
        """
        SELECT * FROM attention_items
        WHERE status IN (:statuses) AND dueAtEpochMillis > :nowEpochMillis
        ORDER BY dueAtEpochMillis ASC
        """,
    )
    suspend fun queryPendingForRestore(
        statuses: List<String>,
        nowEpochMillis: Long,
    ): List<AttentionEntity>

    @Query(
        """
        SELECT * FROM attention_items
        WHERE status IN (:statuses) AND dueAtEpochMillis <= :nowEpochMillis
        ORDER BY dueAtEpochMillis ASC
        """,
    )
    suspend fun queryOverdueForRestore(
        statuses: List<String>,
        nowEpochMillis: Long,
    ): List<AttentionEntity>

    @Query("SELECT * FROM attention_items WHERE sourceKind = :sourceKind")
    suspend fun queryBySourceKind(sourceKind: String): List<AttentionEntity>

    @Query("SELECT COUNT(*) FROM attention_items")
    suspend fun count(): Int

    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun insertAll(entities: List<AttentionEntity>)

    @Upsert
    suspend fun upsert(entity: AttentionEntity)

    @Query(
        """
        UPDATE attention_items
        SET status = :status, actions = '', updatedAtEpochMillis = :updatedAtEpochMillis
        WHERE id = :id AND status IN (:activeStatuses)
        """,
    )
    suspend fun updateTerminalStateIfActive(
        id: String,
        status: String,
        activeStatuses: List<String>,
        updatedAtEpochMillis: Long,
    ): Int

    @Query(
        """
        UPDATE attention_items
        SET status = :status, updatedAtEpochMillis = :updatedAtEpochMillis
        WHERE id = :id
          AND status IN (:pendingStatuses)
          AND dueAtEpochMillis <= :nowEpochMillis
        """,
    )
    suspend fun claimTriggered(
        id: String,
        status: String,
        pendingStatuses: List<String>,
        nowEpochMillis: Long,
        updatedAtEpochMillis: Long,
    ): Int

    @Query(
        """
        UPDATE attention_items
        SET status = :status, dueAtEpochMillis = :dueAtEpochMillis, updatedAtEpochMillis = :updatedAtEpochMillis
        WHERE id = :id AND status IN (:activeStatuses)
        """,
    )
    suspend fun snoozeIfActive(
        id: String,
        status: String,
        dueAtEpochMillis: Long,
        activeStatuses: List<String>,
        updatedAtEpochMillis: Long,
    ): Int

    @Query(
        """
        UPDATE attention_items
        SET status = :failedStatus, actions = '', updatedAtEpochMillis = :updatedAtEpochMillis
        WHERE id = :id
          AND status IN (:pendingStatuses)
          AND dueAtEpochMillis = :dueAtEpochMillis
          AND updatedAtEpochMillis = :expectedUpdatedAtEpochMillis
        """,
    )
    suspend fun markFailedIfPending(
        id: String,
        failedStatus: String,
        pendingStatuses: List<String>,
        dueAtEpochMillis: Long,
        expectedUpdatedAtEpochMillis: Long,
        updatedAtEpochMillis: Long,
    ): Int

    @Query(
        """
        UPDATE attention_items
        SET status = :degradedStatus, updatedAtEpochMillis = :updatedAtEpochMillis
        WHERE id = :id
          AND status IN (:normalPendingStatuses)
          AND dueAtEpochMillis = :dueAtEpochMillis
          AND updatedAtEpochMillis = :expectedUpdatedAtEpochMillis
        """,
    )
    suspend fun markDegradedIfPending(
        id: String,
        degradedStatus: String,
        normalPendingStatuses: List<String>,
        dueAtEpochMillis: Long,
        expectedUpdatedAtEpochMillis: Long,
        updatedAtEpochMillis: Long,
    ): Int

    @Query(
        """
        UPDATE attention_items
        SET status = :waitingStatus, updatedAtEpochMillis = :updatedAtEpochMillis
        WHERE id = :id
          AND status = :degradedStatus
          AND dueAtEpochMillis = :dueAtEpochMillis
          AND updatedAtEpochMillis = :expectedUpdatedAtEpochMillis
        """,
    )
    suspend fun restoreWaitingIfDegraded(
        id: String,
        waitingStatus: String,
        degradedStatus: String,
        dueAtEpochMillis: Long,
        expectedUpdatedAtEpochMillis: Long,
        updatedAtEpochMillis: Long,
    ): Int

    @Query("DELETE FROM attention_items WHERE sourceKind = :sourceKind")
    suspend fun clearBySourceKind(sourceKind: String)
}
