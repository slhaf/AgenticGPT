package work.slhaf.agentic.console.platform.attention

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

class PermissionStateReader {
    fun read(): AndroidPermissionState {
        return AndroidPermissionState(
            notifications = CapabilityState(
                label = "未声明 / 未请求",
                detail = "本阶段不会声明或请求 POST_NOTIFICATIONS，也不会发送系统通知",
                available = false,
            ),
            exactAlarm = CapabilityState(
                label = "未声明 / 不调度",
                detail = "本阶段不会声明精确闹钟权限，也不会调用 AlarmManager",
                available = false,
            ),
            batteryOptimization = CapabilityState("未检测", "后续本地调度阶段再接电池优化检测", false),
            lockScreenInterrupt = CapabilityState("未接入", "强提醒界面和锁屏能力不在本阶段实现", false),
        )
    }
}
