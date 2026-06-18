package work.slhaf.agentic.console.settings

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import work.slhaf.agentic.console.attention.AttentionListStateHolder
import work.slhaf.agentic.console.platform.attention.AndroidPermissionState
import work.slhaf.agentic.console.platform.attention.CapabilityState
import work.slhaf.agentic.console.ui.common.HubConnectionCard
import work.slhaf.agentic.console.ui.common.SettingsSection

@Composable
fun AndroidSettingsScreen(
    permissionState: AndroidPermissionState,
    stateHolder: AttentionListStateHolder,
    onRequestNotificationPermission: () -> Unit,
    onSendTestNotification: () -> Unit,
) {
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(20.dp),
        verticalArrangement = Arrangement.spacedBy(18.dp),
    ) {
        item {
            Text("设置", style = MaterialTheme.typography.headlineSmall)
        }
        item { HubConnectionSection() }
        item { AndroidPermissionSection(permissionState) }
        item { ReminderBehaviorSection() }
        item {
            DebugSection(
                stateHolder = stateHolder,
                onRequestNotificationPermission = onRequestNotificationPermission,
                onSendTestNotification = onSendTestNotification,
            )
        }
    }
}

@Composable
private fun HubConnectionSection() {
    var hubUrl by remember { mutableStateOf("http://127.0.0.1:8080") }
    var token by remember { mutableStateOf("") }
    SettingsSection("Hub 连接") {
        HubConnectionCard(
            hubUrl = hubUrl,
            apiToken = token,
            onHubUrlChange = { hubUrl = it },
            onApiTokenChange = { token = it },
        )
        Text("未连接。本阶段仅保留配置占位，不会发起网络请求。", style = MaterialTheme.typography.bodySmall)
        OutlinedButton(onClick = { }) {
            Text("测试连接（mock，无网络请求）")
        }
    }
}

@Composable
private fun AndroidPermissionSection(permissionState: AndroidPermissionState) {
    SettingsSection("Android 权限") {
        CapabilityRow("通知权限", permissionState.notifications)
        CapabilityRow("精确闹钟", permissionState.exactAlarm)
        CapabilityRow("电池优化", permissionState.batteryOptimization)
        CapabilityRow("锁屏 / 强提醒", permissionState.lockScreenInterrupt)
    }
}

@Composable
private fun ReminderBehaviorSection() {
    SettingsSection("提醒行为") {
        SettingLine("默认 Snooze", "5 分钟")
        SettingLine("Alarm 重复策略", "MVP 不重复")
        SettingLine("默认时区", "按设备本地时区显示；模型保存 epochMillis")
        SettingLine("通知级别", "Reminder 普通，Alarm 强提醒能力仅展示占位")
    }
}

@Composable
private fun DebugSection(
    stateHolder: AttentionListStateHolder,
    onRequestNotificationPermission: () -> Unit,
    onSendTestNotification: () -> Unit,
) {
    SettingsSection("调试") {
        Text("通知按钮只验证 Android 本地通知和通知动作；mock Reminder / Alarm 仍只修改内存数据，不会调度系统 alarm。", style = MaterialTheme.typography.bodySmall)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.fillMaxWidth()) {
            OutlinedButton(onClick = onRequestNotificationPermission, modifier = Modifier.weight(1f)) {
                Text("请求通知权限")
            }
            Button(onClick = onSendTestNotification, modifier = Modifier.weight(1f)) {
                Text("发送测试通知")
            }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.fillMaxWidth()) {
            Button(onClick = stateHolder::createMockReminder, modifier = Modifier.weight(1f)) {
                Text("创建 mock Reminder")
            }
            Button(onClick = stateHolder::createMockAlarm, modifier = Modifier.weight(1f)) {
                Text("创建 mock Alarm")
            }
        }
        OutlinedButton(onClick = stateHolder::clearMockData) {
            Text("清空本地 mock 数据")
        }
    }
}

@Composable
private fun CapabilityRow(title: String, state: CapabilityState) {
    Column(verticalArrangement = Arrangement.spacedBy(2.dp), modifier = Modifier.fillMaxWidth()) {
        Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
            Text(title, style = MaterialTheme.typography.bodyMedium)
            Text(state.label, style = MaterialTheme.typography.labelMedium)
        }
        Text(state.detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

@Composable
private fun SettingLine(title: String, value: String) {
    Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
        Text(title, style = MaterialTheme.typography.bodyMedium)
        Text(value, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}
