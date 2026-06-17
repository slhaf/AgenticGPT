package work.slhaf.agentic.console.attention

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import work.slhaf.agentic.console.domain.attention.AttentionItem
import work.slhaf.agentic.console.domain.attention.AttentionStatus
import work.slhaf.agentic.console.ui.common.AttentionSection
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.TimeZone

@Composable
fun AttentionScreen(stateHolder: AttentionListStateHolder) {
    val state by stateHolder.state.collectAsState()
    var filter by remember { mutableStateOf(AttentionFilter.All) }
    val filtered = state.filtered(filter)
    val waiting = filtered.filter { it.status == AttentionStatus.Waiting || it.status == AttentionStatus.Snoozed }
    val triggered = filtered.filter { it.status == AttentionStatus.Triggered }
    val ended = filtered.filter { it.status in AttentionListUiState.terminalStatuses }
    val timeFormatter = remember { DeviceTimeFormatter() }

    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(20.dp),
        verticalArrangement = Arrangement.spacedBy(18.dp),
    ) {
        item {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("Agentic Attention / 提醒", style = MaterialTheme.typography.headlineSmall)
                Text(
                    text = state.nextItem?.let { "下一次：${timeFormatter.format(it.dueAtEpochMillis)} ${it.title}" } ?: "没有等待中的提醒",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        item {
            StatusOverview(
                waiting = state.waitingCount,
                triggered = state.triggeredCount,
                ended = state.endedCount,
            )
        }
        item {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                AttentionFilter.entries.forEach { item ->
                    FilterChip(
                        selected = filter == item,
                        onClick = { filter = item },
                        label = { Text(item.title) },
                    )
                }
            }
        }
        item {
            AttentionSection(
                title = "正在等待",
                items = waiting,
                dueText = { timeFormatter.format(it.dueAtEpochMillis) },
                onDone = stateHolder::markDone,
                onAcknowledge = stateHolder::acknowledge,
                onSnooze = stateHolder::snooze,
                onCancel = stateHolder::cancel,
            )
        }
        item {
            AttentionSection(
                title = "已触发 / 待处理",
                items = triggered,
                dueText = { "已触发：${timeFormatter.format(it.dueAtEpochMillis)}" },
                onDone = stateHolder::markDone,
                onAcknowledge = stateHolder::acknowledge,
                onSnooze = stateHolder::snooze,
                onCancel = stateHolder::cancel,
            )
        }
        item {
            AttentionSection(
                title = "已结束",
                items = ended,
                dueText = { "结束项：${timeFormatter.format(it.dueAtEpochMillis)}" },
                onDone = stateHolder::markDone,
                onAcknowledge = stateHolder::acknowledge,
                onSnooze = stateHolder::snooze,
                onCancel = stateHolder::cancel,
            )
        }
    }
}

@Composable
private fun StatusOverview(waiting: Int, triggered: Int, ended: Int) {
    Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        OverviewCell("等待中", waiting, Modifier.weight(1f))
        OverviewCell("已触发", triggered, Modifier.weight(1f))
        OverviewCell("已结束", ended, Modifier.weight(1f))
    }
}

@Composable
private fun OverviewCell(label: String, value: Int, modifier: Modifier = Modifier) {
    Surface(modifier = modifier, tonalElevation = 1.dp, shape = MaterialTheme.shapes.small) {
        Column(modifier = Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(text = value.toString(), style = MaterialTheme.typography.titleLarge)
            Text(text = label, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

private class DeviceTimeFormatter {
    private val timeZone = TimeZone.getDefault()
    private val formatter = SimpleDateFormat("MM-dd HH:mm", Locale.getDefault()).apply {
        timeZone = this@DeviceTimeFormatter.timeZone
    }

    fun format(epochMillis: Long): String =
        "${formatter.format(Date(epochMillis))} ${timeZone.getDisplayName(false, TimeZone.SHORT)}"
}
