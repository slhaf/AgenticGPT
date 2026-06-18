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
        WHERE id = :id
        """,
    )
    suspend fun updateTerminalState(
        id: String,
        status: String,
        updatedAtEpochMillis: Long,
    )

    @Query(
        """
        UPDATE attention_items
        SET status = :status, updatedAtEpochMillis = :updatedAtEpochMillis
        WHERE id = :id
        """,
    )
    suspend fun markTriggered(
        id: String,
        status: String,
        updatedAtEpochMillis: Long,
    )

    @Query(
        """
        UPDATE attention_items
        SET status = :status, dueAtEpochMillis = :dueAtEpochMillis, updatedAtEpochMillis = :updatedAtEpochMillis
        WHERE id = :id
        """,
    )
    suspend fun snooze(
        id: String,
        status: String,
        dueAtEpochMillis: Long,
        updatedAtEpochMillis: Long,
    )

    @Query("DELETE FROM attention_items WHERE sourceKind = :sourceKind")
    suspend fun clearBySourceKind(sourceKind: String)
}
