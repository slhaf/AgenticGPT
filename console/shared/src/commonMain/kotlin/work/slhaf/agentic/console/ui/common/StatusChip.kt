package work.slhaf.agentic.console.ui.common

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import work.slhaf.agentic.console.domain.attention.AttentionStatus
import work.slhaf.agentic.console.domain.attention.AttentionType

@Composable
fun TypeChip(type: AttentionType, modifier: Modifier = Modifier) {
    val color = when (type) {
        AttentionType.Reminder -> MaterialTheme.colorScheme.secondaryContainer
        AttentionType.Alarm -> MaterialTheme.colorScheme.errorContainer
    }
    Chip(label = type.name, color = color, modifier = modifier)
}

@Composable
fun StatusChip(status: AttentionStatus, modifier: Modifier = Modifier) {
    val color = when (status) {
        AttentionStatus.Waiting -> MaterialTheme.colorScheme.primaryContainer
        AttentionStatus.Triggered -> MaterialTheme.colorScheme.tertiaryContainer
        AttentionStatus.Snoozed -> MaterialTheme.colorScheme.secondaryContainer
        AttentionStatus.Done,
        AttentionStatus.Acknowledged -> MaterialTheme.colorScheme.surfaceVariant
        AttentionStatus.Cancelled,
        AttentionStatus.Failed -> MaterialTheme.colorScheme.errorContainer
    }
    Chip(label = status.name, color = color, modifier = modifier)
}

@Composable
private fun Chip(label: String, color: Color, modifier: Modifier = Modifier) {
    Box(
        modifier = modifier
            .background(color = color, shape = RoundedCornerShape(8.dp))
            .padding(horizontal = 8.dp, vertical = 4.dp),
    ) {
        Text(text = label, style = MaterialTheme.typography.labelSmall)
    }
}
