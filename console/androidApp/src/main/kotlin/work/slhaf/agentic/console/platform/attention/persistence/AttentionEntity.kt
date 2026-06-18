package work.slhaf.agentic.console.platform.attention.persistence

import androidx.room.Entity
import androidx.room.Index
import androidx.room.PrimaryKey

@Entity(
    tableName = "attention_items",
    indices = [
        Index(value = ["dueAtEpochMillis"]),
        Index(value = ["sourceKind"]),
        Index(value = ["status"]),
    ],
)
data class AttentionEntity(
    @PrimaryKey val id: String,
    val type: String,
    val title: String,
    val message: String?,
    val dueAtEpochMillis: Long,
    val status: String,
    val sourceKind: String,
    val sourceLabel: String,
    val actions: String,
    val scheduleKind: String,
    val reminderDefaultSnoozeMillis: Long?,
    val alarmRequiresAcknowledgement: Boolean?,
    val alarmDefaultSnoozeMillis: Long?,
    val createdAtEpochMillis: Long,
    val updatedAtEpochMillis: Long,
)
