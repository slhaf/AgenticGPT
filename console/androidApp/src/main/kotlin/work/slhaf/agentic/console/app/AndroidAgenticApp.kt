package work.slhaf.agentic.console.app

import android.Manifest
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import work.slhaf.agentic.console.navigation.AgenticNavigationScaffold
import work.slhaf.agentic.console.platform.attention.AndroidAttentionScheduler
import work.slhaf.agentic.console.platform.attention.InMemoryAttentionRepository
import work.slhaf.agentic.console.platform.attention.PermissionStateReader
import work.slhaf.agentic.console.platform.attention.ReminderNotificationService
import work.slhaf.agentic.console.attention.AttentionListStateHolder

@Composable
fun AndroidAgenticApp() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val repository = remember { InMemoryAttentionRepository() }
    val scheduler = remember(context) { AndroidAttentionScheduler(context) }
    val stateHolder = remember { AttentionListStateHolder(repository, scheduler, scope) }
    val permissionStateReader = remember(context) { PermissionStateReader(context) }
    val notificationService = remember(context) { ReminderNotificationService(context) }
    var permissionState by remember(permissionStateReader) { mutableStateOf(permissionStateReader.read()) }
    val refreshPermissionState = { permissionState = permissionStateReader.read() }
    val notificationPermissionLauncher = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.RequestPermission(),
    ) {
        refreshPermissionState()
    }
    val requestNotificationPermission: () -> Unit = {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            notificationPermissionLauncher.launch(Manifest.permission.POST_NOTIFICATIONS)
        } else {
            refreshPermissionState()
        }
    }
    val sendTestNotification: () -> Unit = {
        if (notificationService.canPostNotifications()) {
            notificationService.showTestNotification()
            refreshPermissionState()
        } else {
            requestNotificationPermission()
        }
    }

    LaunchedEffect(notificationService) {
        notificationService.ensureChannel()
        refreshPermissionState()
    }

    val destinations = remember(stateHolder, permissionState) {
        buildAndroidDestinations(
            stateHolder = stateHolder,
            permissionState = permissionState,
            onRequestNotificationPermission = requestNotificationPermission,
            onSendTestNotification = sendTestNotification,
        )
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
