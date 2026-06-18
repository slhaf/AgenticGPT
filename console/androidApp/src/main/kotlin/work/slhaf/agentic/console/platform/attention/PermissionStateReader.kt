package work.slhaf.agentic.console.platform.attention

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build

data class AndroidPermissionState(
    val notifications: CapabilityState,
    val exactAlarm: CapabilityState,
    val batteryOptimization: CapabilityState,
    val lockScreenInterrupt: CapabilityState,
)

data class CapabilityState(
    val label: String,
    val detail: String,
    val available: Boolean,
)

class PermissionStateReader(
    context: Context,
) {
    private val appContext = context.applicationContext

    fun read(): AndroidPermissionState {
        return AndroidPermissionState(
            notifications = notificationState(),
            exactAlarm = CapabilityState(
                label = "未接入",
                detail = "第一阶段不接精确闹钟权限；Snooze 仅使用最小 inexact AlarmManager 调度",
                available = false,
            ),
            batteryOptimization = CapabilityState("未检测", "本地通知 spike 不处理电池优化", false),
            lockScreenInterrupt = CapabilityState("未接入", "强提醒界面和锁屏能力不在本阶段实现", false),
        )
    }

    private fun notificationState(): CapabilityState {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
            return CapabilityState(
                label = "无需运行时授权",
                detail = "Android 12L 及以下不需要 POST_NOTIFICATIONS 运行时权限",
                available = true,
            )
        }

        val granted = appContext.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED

        return if (granted) {
            CapabilityState(
                label = "已授权",
                detail = "可以发送 Android 本地通知",
                available = true,
            )
        } else {
            CapabilityState(
                label = "未授权",
                detail = "点击调试区的请求通知权限后，才会发送测试通知",
                available = false,
            )
        }
    }
}
