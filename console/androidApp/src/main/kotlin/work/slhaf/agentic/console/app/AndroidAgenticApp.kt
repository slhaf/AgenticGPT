package work.slhaf.agentic.console.app

import android.Manifest
import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import work.slhaf.agentic.console.navigation.AgenticNavigationScaffold
import work.slhaf.agentic.console.platform.attention.AndroidAttentionScheduler
import work.slhaf.agentic.console.platform.attention.PermissionStateReader
import work.slhaf.agentic.console.platform.attention.ReminderNotificationService
import work.slhaf.agentic.console.attention.AttentionListStateHolder
import work.slhaf.agentic.console.platform.attention.persistence.AndroidRoomAttentionRepository
import work.slhaf.agentic.console.platform.attention.persistence.AttentionDatabase

@Composable
fun AndroidAgenticApp() {
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current
    val scope = rememberCoroutineScope()
    val database = remember(context) { AttentionDatabase.create(context) }
    val repository = remember(database, scope) { AndroidRoomAttentionRepository(database.attentionDao(), scope) }
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
    val openExactAlarmSettings: () -> Unit = {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            val packageUri = Uri.parse("package:${context.packageName}")
            val exactAlarmIntent = Intent(Settings.ACTION_REQUEST_SCHEDULE_EXACT_ALARM, packageUri)
            try {
                context.startActivity(exactAlarmIntent)
            } catch (_: ActivityNotFoundException) {
                val appSettingsIntent = Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, packageUri)
                context.startActivity(appSettingsIntent)
            }
        } else {
            refreshPermissionState()
        }
    }

    LaunchedEffect(notificationService) {
        notificationService.ensureChannel()
        refreshPermissionState()
    }

    DisposableEffect(lifecycleOwner, permissionStateReader) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_RESUME) {
                refreshPermissionState()
            }
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose {
            lifecycleOwner.lifecycle.removeObserver(observer)
        }
    }

    val destinations = remember(stateHolder, permissionState) {
        buildAndroidDestinations(
            stateHolder = stateHolder,
            permissionState = permissionState,
            onRequestNotificationPermission = requestNotificationPermission,
            onSendTestNotification = sendTestNotification,
            onOpenExactAlarmSettings = openExactAlarmSettings,
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
