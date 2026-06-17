package work.slhaf.agentic.console

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.Composable
import androidx.compose.ui.tooling.preview.Preview
import work.slhaf.agentic.console.app.AndroidAgenticApp

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)

        setContent {
            AndroidAgenticApp()
        }
    }
}

@Preview
@Composable
fun AppAndroidPreview() {
    AndroidAgenticApp()
}
