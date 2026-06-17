package work.slhaf.agentic.console.navigation

import androidx.compose.runtime.Composable

data class AppDestination(
    val id: String,
    val title: String,
    val icon: AppIcon,
    val kind: DestinationKind,
    val content: @Composable () -> Unit,
)

enum class DestinationKind {
    Primary,
    Utility,
}

enum class AppIcon(val glyph: String) {
    Bell("!"),
    Settings("*"),
    Console("#"),
}
