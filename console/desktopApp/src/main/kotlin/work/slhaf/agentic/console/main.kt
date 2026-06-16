package work.slhaf.agentic.console

import androidx.compose.ui.window.Window
import androidx.compose.ui.window.application

fun main() = application {
    Window(
        onCloseRequest = ::exitApplication,
        title = "Agentic Console",
    ) {
        App()
    }
}