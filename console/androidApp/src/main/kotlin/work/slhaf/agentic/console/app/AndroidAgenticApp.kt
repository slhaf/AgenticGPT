package work.slhaf.agentic.console.app

import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import work.slhaf.agentic.console.navigation.AgenticNavigationScaffold
import work.slhaf.agentic.console.platform.attention.InMemoryAttentionRepository
import work.slhaf.agentic.console.platform.attention.MockAttentionScheduler
import work.slhaf.agentic.console.platform.attention.PermissionStateReader
import work.slhaf.agentic.console.attention.AttentionListStateHolder

@Composable
fun AndroidAgenticApp() {
    val scope = rememberCoroutineScope()
    val repository = remember { InMemoryAttentionRepository() }
    val scheduler = remember { MockAttentionScheduler() }
    val stateHolder = remember { AttentionListStateHolder(repository, scheduler, scope) }
    val permissionState = remember { PermissionStateReader().read() }
    val destinations = remember(stateHolder, permissionState) {
        buildAndroidDestinations(stateHolder = stateHolder, permissionState = permissionState)
    }
    var selectedId by remember { mutableStateOf(destinations.first().id) }
    val selected = destinations.firstOrNull { it.id == selectedId } ?: destinations.first()

    MaterialTheme {
        AgenticNavigationScaffold(
            destinations = destinations,
            selectedId = selected.id,
            onSelect = { selectedId = it },
            content = selected.content,
        )
    }
}
