# Console 开发说明

这是一个 Kotlin Multiplatform 项目，当前配置的目标平台为 Android、Web 和 Desktop（JVM）；当前 Gradle 构建配置尚未启用 iOS 编译目标。

- [/shared](./shared/src) 存放 Compose Multiplatform 应用之间共享的代码，其中包含若干源集（source set）：
  - [commonMain](./shared/src/commonMain/kotlin) 存放所有目标共用的代码。
  - 其他源集存放只针对相应平台编译的 Kotlin 代码。例如，若以后配置 iOS 编译目标，iOS 专属代码应放在相应的 iOS 源集；Desktop（JVM）专属代码则应放在 [jvmMain](./shared/src/jvmMain/kotlin)。

### 运行应用

建议使用 IDE 工具栏中的运行配置启动应用。也可以使用以下 Gradle 命令：

- Android：使用 IDE 运行配置启动应用。`./gradlew :androidApp:assembleDebug` 只构建 APK，不会安装或启动应用。
- Desktop：
  - 热重载：`./gradlew :desktopApp:hotRun --auto`
  - 标准运行：`./gradlew :desktopApp:run`
- Web：
  - Wasm 目标（速度更快，适用于现代浏览器）：`./gradlew :webApp:wasmJsBrowserDevelopmentRun`
  - JS 目标（速度较慢，兼容较旧浏览器）：`./gradlew :webApp:jsBrowserDevelopmentRun`

### 运行测试

使用 IDE 编辑器侧栏中的运行按钮，或执行以下 Gradle 任务：

- Android 测试：`./gradlew :shared:testAndroidHostTest`
- Desktop 测试：`./gradlew :shared:jvmTest`
- Web 测试：
  - Wasm 目标：`./gradlew :shared:wasmJsTest`
  - JS 目标：`./gradlew :shared:jsTest`

---

了解更多：[Kotlin Multiplatform](https://www.jetbrains.com/help/kotlin-multiplatform-dev/get-started.html)、[Compose Multiplatform](https://github.com/JetBrains/compose-multiplatform/#compose-multiplatform)、[Kotlin/Wasm](https://kotl.in/wasm/)。

欢迎在公共 Slack 频道 [#compose-web](https://slack-chats.kotlinlang.org/c/compose-web) 分享 Compose/Web 和 Kotlin/Wasm 的使用反馈。如遇问题，请通过 [YouTrack](https://youtrack.jetbrains.com/newIssue?project=CMP) 提交。
