package work.slhaf.agentic.console.ui.common

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import work.slhaf.agentic.console.domain.attention.AttentionAction
import work.slhaf.agentic.console.domain.attention.AttentionItem

@Composable
fun AttentionCard(
    item: AttentionItem,
    dueText: String,
    onDone: (String) -> Unit,
    onAcknowledge: (String) -> Unit,
    onSnooze: (String) -> Unit,
    onCancel: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    Card(
        modifier = modifier.fillMaxWidth(),
        shape = RoundedCornerShape(8.dp),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface),
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
    ) {
        Column(
            modifier = Modifier.padding(14.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                TypeChip(item.type)
                StatusChip(item.status)
            }
            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(text = dueText, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.primary)
                Text(text = item.title, style = MaterialTheme.typography.titleMedium)
                item.message?.let {
                    Text(text = it, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
            if (item.actions.isNotEmpty()) {
                FlowRow(
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    if (AttentionAction.Done in item.actions) {
                        Button(onClick = { onDone(item.id) }) {
                            Text("完成")
                        }
                    }
                    if (AttentionAction.Acknowledge in item.actions) {
                        Button(onClick = { onAcknowledge(item.id) }) {
                            Text("我已处理")
                        }
                    }
                    if (AttentionAction.Snooze in item.actions) {
                        OutlinedButton(onClick = { onSnooze(item.id) }) {
                            Text("稍后")
                        }
                    }
                    if (AttentionAction.Cancel in item.actions) {
                        OutlinedButton(onClick = { onCancel(item.id) }) {
                            Text("取消")
                        }
                    }
                }
            }
        }
    }
}
