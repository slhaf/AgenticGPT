package work.slhaf.agentic.console.ui.common

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import work.slhaf.agentic.console.domain.attention.AttentionItem

@Composable
fun AttentionSection(
    title: String,
    items: List<AttentionItem>,
    dueText: (AttentionItem) -> String,
    onDone: (String) -> Unit,
    onAcknowledge: (String) -> Unit,
    onSnooze: (String) -> Unit,
    onCancel: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(modifier = modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Text(text = title, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        if (items.isEmpty()) {
            EmptyState("没有$title")
        } else {
            items.forEach { item ->
                AttentionCard(
                    item = item,
                    dueText = dueText(item),
                    onDone = onDone,
                    onAcknowledge = onAcknowledge,
                    onSnooze = onSnooze,
                    onCancel = onCancel,
                )
            }
        }
    }
}
