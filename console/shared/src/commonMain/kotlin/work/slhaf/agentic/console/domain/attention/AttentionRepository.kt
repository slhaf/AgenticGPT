package work.slhaf.agentic.console.domain.attention

import kotlinx.coroutines.flow.StateFlow
import kotlin.time.Duration

interface AttentionRepository {
    fun observeItems(): StateFlow<List<AttentionItem>>
    suspend fun create(item: AttentionItem)
    suspend fun markDone(id: String)
    suspend fun acknowledge(id: String)
    suspend fun snooze(id: String, duration: Duration)
    suspend fun cancel(id: String)
    suspend fun clearMockData()
}
