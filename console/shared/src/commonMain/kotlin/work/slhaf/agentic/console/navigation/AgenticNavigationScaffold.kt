package work.slhaf.agentic.console.navigation

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

@Composable
fun AgenticNavigationScaffold(
    destinations: List<AppDestination>,
    selectedId: String,
    onSelect: (String) -> Unit,
    content: @Composable () -> Unit,
) {
    Scaffold(
        bottomBar = {
            NavigationBar {
                destinations.forEach { destination ->
                    NavigationBarItem(
                        selected = destination.id == selectedId,
                        onClick = { onSelect(destination.id) },
                        icon = { Text(destination.icon.glyph, style = MaterialTheme.typography.titleMedium) },
                        label = { Text(destination.title) },
                    )
                }
            }
        },
    ) { innerPadding ->
        Box(
            modifier = Modifier
                .fillMaxSize()
                .padding(innerPadding),
        ) {
            content()
        }
    }
}

@Composable
fun AgenticPlaceholderApp() {
    MaterialTheme {
        Column(modifier = Modifier.fillMaxSize()) {
            Text(
                text = "Agentic Console",
                style = MaterialTheme.typography.headlineMedium,
                modifier = Modifier.padding(horizontal = 24.dp, vertical = 32.dp),
            )
            Text(
                text = "Platform-specific destinations are registered by each app target.",
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.padding(horizontal = 24.dp),
            )
        }
    }
}
