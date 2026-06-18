package work.slhaf.agentic.console.app

import work.slhaf.agentic.console.attention.AttentionListStateHolder
import work.slhaf.agentic.console.attention.AttentionScreen
import work.slhaf.agentic.console.navigation.AppDestination
import work.slhaf.agentic.console.navigation.AppIcon
import work.slhaf.agentic.console.navigation.DestinationKind
import work.slhaf.agentic.console.platform.attention.AndroidPermissionState
import work.slhaf.agentic.console.settings.AndroidSettingsScreen

fun buildAndroidDestinations(
    stateHolder: AttentionListStateHolder,
    permissionState: AndroidPermissionState,
    onRequestNotificationPermission: () -> Unit,
    onSendTestNotification: () -> Unit,
): List<AppDestination> = listOf(
    AppDestination(
        id = "attention",
        title = "提醒",
        icon = AppIcon.Bell,
        kind = DestinationKind.Primary,
    ) {
        AttentionScreen(stateHolder)
    },
    AppDestination(
        id = "settings",
        title = "设置",
        icon = AppIcon.Settings,
        kind = DestinationKind.Utility,
    ) {
        AndroidSettingsScreen(
            permissionState = permissionState,
            stateHolder = stateHolder,
            onRequestNotificationPermission = onRequestNotificationPermission,
            onSendTestNotification = onSendTestNotification,
        )
    },
)
