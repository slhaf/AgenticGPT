# 邮件渲染调查结果

## 现有实现和真实邮件

- 输入为 `/home/dev/.agentic_gpt/workspace/email-render-repro-20261105/` 中的两封原始 `.eml`。对照在远端 Laptop 进行。
- 现有 litehtml 绘图路径对内在尺寸大于显示尺寸的图片未做缩放，按显示框裁切；两封邮件还暴露出布局或字体差异。Chromium 对照显示图片缩放和 CI 邮件卡片正常。
- 当前路径对 HTML 有预处理；用户明确要求直接渲染原始 HTML。当前工作是调查，尚未更改代码。
- Room 环境曾观察到缺少字体，提供西文与中文字体后文字恢复。浏览器方案也需要可用字体。

## Chromium 观察

- 两封原始邮件用远端 Chromium 渲染，远程图片分别加载 4/4 和 1/1。
- 在远端 Laptop 顺序复用浏览器的单次测量：渲染分别约 1.08 秒和 1.45 秒，包含等待网络；Chromium 进程峰值按 PSS 分摊约 364 MiB 和 376 MiB，分别有 8 和 9 个 Chromium 进程。启动约 0.09 秒。机器内存 15 GiB；未测 CPU、并发或 Room 上的表现。
- 现有 worker 设置 `RLIMIT_AS=512 MiB`，这是虚拟地址空间限制，不能假设可原样用于 Chromium。浏览器、系统库和字体的交付方式尚待确定。
- Chromium 对照图片位于 `/home/dev/.codex/visualizations/2026/11/05/01a1a917-adbd-7ef3-8658-787b3e9835d3/email-render-aerdnx30/`，分别为 `github-ci-chromium.png` 和 `github-pr-review-chromium.png`。

## Blitz 观察

- [Blitz 官方项目](https://github.com/DioxusLabs/blitz)当前标注 beta；[元素状态](https://blitz.is/status/elements)显示表格布局使用 CSS Grid 模拟，部分表格元素不支持。WPT 结果是标准测试，不是邮件专项验证。
- 使用预编译 `blitz-py 0.6.0`，其依赖 Blitz `0.3.0-beta.4`，以原始 HTML、1024 像素宽在 Laptop 运行。封装不自行联网：通过 Chromium 取得原邮件中 4/4 和 1/1 个远程图片资源，再交给封装。
- CI 邮件的标题和页脚出现，但状态卡片及其文字缺失；审批邮件正文和中文可读。此结果不能区分是 Blitz 本身还是第三方封装所致。
- 同次测量的渲染调用约 36 ms、10 ms，Python 进程峰值约 55 MiB；不包含网络资源获取的浏览器开销，且 CI 结果缺失内容，不能作为完整渲染对比。
- 输出图片在上述图片目录中，名称分别为 `github-ci-blitz-py-remote.png` 和 `github-pr-review-blitz-py-remote.png`。官方 CPU 截图示例与原生网络路径未对原始邮件实测。

## HtmlUnit 与 Playwright

- [HtmlUnit](https://github.com/HtmlUnit/htmlunit) 是 Java 的无界面浏览器模拟器，主要用于测试和数据提取。它的 [WebDriver 元素截图实现](https://github.com/SeleniumHQ/htmlunit-driver/blob/master/src/main/java/org/openqa/selenium/htmlunit/HtmlUnitWebElement.java)明确不支持截图，尚未找到可直接将页面渲染为图片的官方能力，不适合作为本任务的截图引擎。
- [Playwright 官方支持的语言](https://playwright.dev/docs/languages)包括 JavaScript/TypeScript、Python、Java 和 .NET，没有官方 Rust 绑定。它是浏览器自动化工具，依赖 Chromium、Firefox 或 WebKit 执行实际渲染，无法消除浏览器的资源开销。
- 非官方 [playwright-rs](https://github.com/padamson/playwright-rust) 提供 Rust API；其文档说明通过 Node.js Playwright 驱动控制浏览器，驱动由项目提供，仍需浏览器及其依赖。没有在本任务中实测该 Rust 绑定。
- [chromiumoxide](https://github.com/mattsse/chromiumoxide) 是 Rust 的 Chromium 控制库，直接通过 Chrome DevTools Protocol 控制浏览器，无需 Playwright 驱动；仍需 Chromium。这是接入候选而非已确定方案，未用此库运行原始邮件。

## Rust 原生浏览器引擎

- [Servo](https://github.com/servo/servo) 是 Rust 浏览器引擎，支持嵌入式 WebView，其 [take_screenshot 接口](https://doc.servo.org/servo/struct.WebView.html)可等待图片、字体和页面准备后截图；也有 Linux x86_64、ARM64 的预编译 nightly。
- 在 Laptop 用 Servo 0.8.0-fde5631b6 官方 x86_64 nightly，以 1024×768 视口直接渲染两封原始邮件的 HTML：CI 状态卡片、图标和任务列表可见，审批邮件内容可见，但中文标点出现方框，原因未确定。结果分别为图片目录中的 `github-ci-servo.png` 和 `github-pr-review-servo.png`。
- Servo 两次独立进程从启动到输出分别约 1.21 秒和 1.32 秒；两次子进程的累计最大 RSS 约 422 MiB。这与此前 Chromium 的 PSS 口径不同，不可直接相减比较。仅用 768 像素视口测试了当前两封邮件；长页面截图、远程资源完整性、实际嵌入接口、ARM64/musl 和 Room 环境均未验证。
- 审批邮件方框对应顿号 `、`（U+3001）和句号 `。`（U+3002）。Laptop 已安装的 Noto Serif 缺少这些字符，而 Noto Serif CJK 覆盖这些字符。使用仅影响本次进程的 fontconfig 配置，把默认 `serif` 映射到 Noto Serif CJK SC，在不改写原邮件 HTML 的情况下，Servo 的两种标点恢复；结果为图片目录的 `github-pr-review-servo-fontconfig.png`。对 CI 原邮件同样运行了该配置，卡片正常，图片为 `github-ci-servo-fontconfig.png`。
- 这个配置改变了默认衬线字体，因此可能改变字体外观、换行或分页；不能当作保持所有邮件排版完全不变的通用字形回退修复。邮件显式指定字体时的缺字回退、其他文字及其他部署环境尚未验证。另用强制指定字体的外部样式证明字体能够显示，但该诊断会覆盖邮件字体，不作为修复方案。
- [Gosub](https://github.com/gosub-io/gosub-engine) 是可嵌入的 Rust 浏览器引擎，项目列有网络、表格布局和 CPU 渲染后端；当前仍在开发，脚本尚未接入。未验证原始邮件和资源占用。
- [Lightpanda](https://github.com/lightpanda-io/docs/blob/main/src/content/core-concepts/architecture-overview.mdx) 用 Zig 实现，没有真正的图形渲染管线，截图为占位图，不适合 HTML 转图片。

## 进一步网络筛选

- [Gosub 无头截图](https://github.com/gosub-io/gosub-engine/blob/main/docs/headless.md)提供完整页面 PNG 路径；项目文档列出独立表格布局、图片和字体加载，但仍在开发，未验证两封邮件、性能或部署。
- [FastRender](https://github.com/wilsonzlin/fastrender)提供 Rust HTML 到图片 API 和表格布局，项目明确称 API 不稳定、不建议用于生产；未对本任务实测。
- [OxidePage](https://github.com/n0madic/oxidepage)是独立 Rust 无头引擎，列出 PNG 和整页截图能力；项目自述处于早期阶段，没有已发布的二进制或 crate。未对本任务实测。
- [OxiBrowser](https://github.com/project-oxi/oxibrowser/blob/main/docs/ARCHITECTURE.md)的实际截图路径使用 Blitz，因此不是独立于 Blitz 的渲染引擎；不能据名称推断会解决已观察到的 Blitz 封装缺失内容问题。
- [WebKitGTK](https://webkitgtk.org/reference/webkit2gtk/stable/method.WebView.get_snapshot.html)可通过 Rust 绑定调用 WebKit 内核并获取页面快照，但内核不是 Rust 实现，依赖系统原生库和图形环境；无头接入、发布、资源占用和两封邮件的效果均未验证。

## 社区讨论

- [Rust 社区关于 Blitz 与 Servo 的讨论](https://www.reddit.com/r/rust/comments/1q5ky5w/why_we_are_building_a_new_browser_from_scratch/)中，Blitz 维护者提出 Blitz 适合嵌入和 HTML 邮件、Servo 更侧重完整浏览器功能。这是维护者观点，不能代替真实邮件验证；目前实测的是第三方 Blitz 封装。
- [Iced WebView 开发者的体验](https://gofranz.com/blog/web-rendering-in-iced-what-actually-works/)中，作者比较了 litehtml、Blitz、Servo 和 Chromium Embedded Framework；2026-02 的个人体验为 Servo 有不稳定场景、CEF 更成熟。这是特定版本和 GUI 集成下的经验，不等同于本任务的无头表现。
- [Obscura](https://github.com/h4ckf0r0day/obscura)有 Rust 无头浏览器和独立图形渲染管线，官方列出表格、图片、整页截图等能力，但其官方承认与 Chromium 仍有差异。[社区讨论](https://www.reddit.com/r/selfhosted/comments/1vid4lg/lightweight_headless_browser_with_native/)主要包括作者介绍、用户关注和个别经验，并非针对邮件的独立评测；项目给出的轻量性能数字也不能直接视为完整截图结果。尚未在两封邮件上测试。
- 开源邮件转换工具 [emailtopdf](https://github.com/iris-ng/emailtopdf)使用 Playwright/Chromium，也讨论 WeasyPrint。它提供的是该工具的工程选择与报告，并非 Rust 原生渲染器或本任务的评测。

## Obscura 远端实测与决策

- 用户指定两封原始邮件，在 Laptop 用 Obscura 0.2.6 的发布程序，通过 CDP 直接控制页面和截图；未使用 Playwright 或 Chromium。原始 HTML 通过远端临时本机 HTTP 服务提供，视口 1024×768，等待图片报告加载完成。
- 加载远端已有的 Noto Sans CJK 和 Noto Serif CJK 字体文件后，CI 邮件主要卡片与图标可见，审批邮件中文可读。CI 页脚的 `・` 分隔符显示为方框，排版与 Chromium 有差异。图片为 `github-ci-obscura-fonts.png` 和 `github-pr-review-obscura-fonts.png`。
- 单次顺序复用同一进程，CDP 操作至截图分别约 0.56 秒和 0.80 秒；50 毫秒采样得到的进程 PSS 峰值分别约 40 MiB 和 91 MiB，图片加载状态分别为 4/4 和 1/1。结果仅覆盖这次运行，不包含测试控制进程，不是 Rust 库嵌入实测，也不是吞吐量或 CPU 基准。
- 第一次尝试通过符号链接传入字体未生效，源码显示字体目录扫描跳过符号链接；改用实际字体文件后中文可见。[项目说明](https://github.com/h4ckf0r0day/obscura/blob/main/docs/Use-as-a-Rust-library.md)有可嵌入的 Rust API，正式集成未验证。
- 经核对 [v0.2.6 字体加载源码](https://github.com/h4ckf0r0day/obscura/blob/v0.2.6/crates/obscura-render/src/inline.rs)：额外字体以目录为单位递归扫描，支持 TTF/TTC/OTF/OTC，跳过符号链接；系统字体不会自动扫描。`configure_font_directories` 使用进程级 `OnceLock`，必须在首次渲染前配置，初始化后不能通过该接口重新设置，若要变更该配置必须考虑进程生命周期。
- 调查方式纠正：测试时由我额外挑选并复制两份字体到临时目录，没有先直接尝试让 Obscura 加载设备实际系统字体目录。这是测试过程人为收窄字体集合，不是引擎要求逐个指定或复制字体文件；它导致的繁琐流程和字体效果不应归因于 Obscura 本身。输出及资源数据只能描述该次指定字体集合下的表现，实际系统字体目录的效果未验证。用户基于当时信息决定排除 Obscura，该决定不自行改变。
- 用户认为测试过程过于麻烦，明确决定排除 Obscura；停止继续测试和选型。

## Obscura 系统字体目录重测

- 根据用户新要求，在 Laptop 对两封原始邮件重测 Obscura 0.2.6；直接提供系统存在的 `/usr/share/fonts` 和 `/home/slhaf/.local/share/fonts`，没有挑选或复制字体，原始邮件 HTML 未改写，远程图片正常加载。目录分别含 952 个（1,084.1 MiB）和 4 个（2.4 MiB）字体文件，均没有字体文件或目录符号链接。
- 两封邮件在同一进程顺序渲染，视口为 1024×768。按 50 ms 采样的进程 PSS 峰值分别为 1,120.7 MiB 和 1,136.8 MiB；耗时分别为 1.708 秒和 0.819 秒，首次包含字体加载；引擎报告图片加载分别为 4/4 和 1/1。与之前只加载两份字体所得的 40/91 MiB 不同，此次为系统字体目录的实际结果。Obscura 字体加载代码将目录中的字体文件读入字体库，字体文件总量解释了这次较高的内存占用。
- 两封正文、CI 卡片和中文均可见，先前 CI 页脚分隔符的方框消失；整体布局与 Chromium 接近，但字体和间距有差异。结果文件为 `github-ci-obscura-system-fonts.png` 和 `github-pr-review-obscura-system-fonts.png`，与 Chromium 对照位于同一图片目录。
- 同一远端此前测得 Chromium PSS 分别约 364 MiB 和 376 MiB；其计时包含 networkidle 等待，时间口径不同。此次使用发布程序及直接 CDP 请求截图，未使用 Playwright；正式 Rust 库集成仍未进行。

## Obscura 字体按需加载调查

- [v0.2.6 源码](https://github.com/h4ckf0r0day/obscura/blob/v0.2.6/crates/obscura-render/src/inline.rs)在首次渲染时扫描配置的目录，将每个字体文件 `std::fs::read` 后以 `Source::Binary` 注册；进程缓存保存这些数据。公开的目录配置接口无法选择按需模式，默认也不自动发现系统字体。
- Obscura 内置的 cosmic-text 0.14.2 使用 fontdb 0.16.2，其 `std` feature 已开启 fontdb 的 `memmap`（包含 fs）；[fontdb 源码](https://github.com/RazrFalcon/fontdb/blob/v0.16.2/src/lib.rs)支持 `load_font_source(Source::File(path))`，先映射文件并解析字体元数据，保存文件来源，字形数据由文件映射读取。cosmic-text 获取字体时会共享映射并缓存实际访问过的字体。把 Obscura 的 Binary 注册路径改为 File 是通用的候选修复，不筛选字体、不改写 HTML；初始化仍需遍历字体及提取元数据，当前 `register_loaded_faces` 还会读取每个字体的度量信息，不能称为完全无扫描或承诺具体内存数值。
- 字体发现和匹配是独立问题：Obscura 当前使用自己的通用字体映射和 cosmic-text 平台回退列表，并将 locale 传为 en-US；仅改 File 无法保证遵循系统字体配置或字体回退顺序。fontdb 的 `load_system_fonts` 也不是完整系统匹配接口。Linux 上可考虑通过 [Fontconfig 匹配接口](https://fontconfig.pages.freedesktop.org/fontconfig/fontconfig-devel/)获取系统匹配及覆盖回退，按匹配结果结合文件映射；这需要接入渲染和缺字回退路径，而非只改启动参数。
- 上述候选为源码分析，尚未修改或编译 Obscura，也没有在两封原始邮件上测量修改后的图片和内存；因此不报告预估收益为实测结果。

- 补查上游方案：贡献者的 [PR #880](https://github.com/h4ckf0r0day/obscura/pull/880)曾提出 `fontdb::Database::load_fonts_dir` 路线，该库在开启 memmap 时用文件映射；截至查询时该 PR 仍未合并。上游已合并的 [PR #966](https://github.com/h4ckf0r0day/obscura/pull/966)采用了当前目录扫描和 Binary 内存缓存实现。两项 PR 都没有自动读取系统字体；前者说明这是为了保持跨机器布局的确定性。因此文件映射是上游贡献者提出过的方案，但不能说是当前发布版本已经提供的模式，也不能推断维护者拒绝该方案的原因。

## Obscura 字体文件映射远端试验

- 用户要求直接尝试上游提案的文件映射路径。为保证可与上次对照，远端编译 Obscura v0.2.6，将 `load_font_directories` 中的 `std::fs::read` + `Source::Binary` 替换为 `Source::File(path)`，保留原有字体发现及匹配。该试验实现与上游未合并 PR #880 中 fontdb 文件映射方向相同，但不是直接运行该 PR 分支；没有修改 AgenticGPT 产品代码。
- 原始邮件 HTML 和系统字体目录（`/usr/share/fonts`、`/home/slhaf/.local/share/fonts`）与前次相同，1024×768，顺序复用一个进程，50 ms 采样 PSS。CI 邮件：67.3 MiB、1.439 秒、图片加载 4/4；审批邮件：59.1 MiB、0.843 秒、图片加载 1/1。前次 Binary 加载分别为 1120.7 MiB 与 1136.8 MiB；前次计时 1.708 秒与 0.819 秒。
- 新生成的两张 PNG 与此前对应的系统字体 Binary 版本逐文件 SHA-256 相同，图片内容字节完全一致。结果为同一图片目录的 `github-ci-obscura-mmap.png`、`github-pr-review-obscura-mmap.png`。该试验使用 CDP 测试发布程序等价接口，尚未在 AgenticGPT 中集成。

## 自维护 Obscura 对构建和发版的影响

- 用户同意收敛到 Obscura 单引擎。以 Rust 依赖嵌入时无需另行交付 Obscura CLI；当前邮件渲染已有通过同一个 agentic-gpt 可执行文件启动的 worker。若采用本地修改，发布构建必须能获得修改后的源码，可将源码纳入 Agentic 仓库或锁定可访问的 Git 版本，不能依赖开发机临时目录。
- Agentic 现有发行链通过 `scripts/dist-linux.sh` 构建 x86_64、aarch64 的 GNU 与静态 musl 四个目标并打包三个程序。Obscura 的 browser/render 接入引入 `deno_core` 和 V8；其 v0.2.6 官方 Linux 发布仅列 x86_64 GNU 与 aarch64 GNU。其 `obscura-js/build.rs` 对 host/target 不一致使用跨平台 snapshot 路径，找不到工具时退回 host snapshot 并提示目标设备可能崩溃；这对 Agentic 的交叉构建是必须具体验证的问题。V8 构建脚本还按目标下载预编译静态库或从源码构建。
- 远端此前仅完成修改版 Obscura 原生 x86_64 GNU 编译与运行，release 构建耗时 1 分 51 秒；没有执行 Agentic 集成、ARM64 或 musl 构建，不能据此断言四目标可直接发布。四目标兼容、实际产物体积和运行效果需在正式集成时通过原有发行链确认。

## 自维护 Obscura 的许可证影响

- Obscura v0.2.6 根协议为 Apache-2.0，Agentic 主代码为 MIT；Apache-2.0 允许修改、静态链接和发行，Agentic 原创部分可保留 MIT。重新分发时应向接收者提供 Apache-2.0 许可证，在修改过的 Obscura 文件标注修改，保留适用的上游版权和归属；如果上游含适用的 NOTICE，应同时保留其所需归属。该版本根目录没有 NOTICE。协议不要求把修改提交给上游。
- Obscura 渲染器通过 include_bytes 编入字体，包括 Noto Color Emoji；该字体随源码的许可为 SIL OFL 1.1，二进制随软件分发字体时需要带许可及版权声明。其他实际编入字体和 V8 等依赖也各有许可，集成后须按最终实际依赖提供对应声明。
- 当前 `.github/workflows/release.yml` 的发行 tar 只包含三个可执行文件；如果将这些依赖编入 Agent，需要调整发行包，让接收者同时收到所需的许可证与声明。不得仅依赖源码仓库保留许可证。
- 该版本 `assets` 明确提供了 Noto Color Emoji 的来源和许可文本；Liberation 与 DejaVu 内置字体存在于 assets，但同目录未见它们各自的来源及许可文件，实际发行前需要核实并补齐适用声明。不能因为运行时加载系统字体就认为程序不包含内置字体。

## 单引擎渲染生命周期

- 当前 Agentic 每个邮件渲染请求/切片启动同一 agentic-gpt 可执行文件的 `__email-render-worker`，子进程处理一个请求后退出；父进程正常、错误和超时路径等待回收，调用取消时也有 kill 和等待回收的处理。Obscura 作为库放进该 worker，可以按现有进程所有权释放 V8、页面和字体缓存，无需引入独立常驻服务。
- 放弃引擎选择配置后没有引擎切换所需的热重载生命周期；每次新 worker 读取当次需要的配置，已开始的请求使用启动时的值。实际已有邮件配置仍按既有机制更新，不因此改变其语义。单次进程会重复引擎和字体索引的启动成本；之前 67.3/59.1 MiB 测试顺序复用进程，第二封时间是热进程数据，不能当作每次独立 worker 的时间。
- 当前 worker 有既存 `RLIMIT_AS=512 MiB`、`RLIMIT_CPU=20s` 和 30 秒超时；512 MiB 限制的是虚拟地址空间，不是之前测得的 PSS，V8 与字体映射能否在该限制下运行需要实际确认。用户要求不得自行增加控制措施，不能无依据新增限制；已有控制如不适用，需要明确说明调整。

- Obscura 的页面与 V8 runtime 具有线程归属；实现时应在同一线程创建、使用和销毁页面，worker 内使用单线程运行时即可。字体目录的进程级 OnceLock 需要在首次渲染前配置；一次性 worker 随请求结束退出，新请求自然重新初始化。

## imageWithRemote 的定位

- 用户询问它是否还有存在依据：现有 `image` 明确不联网，`imageWithRemote` 由 Agent 先发现并下载部分远程图片，且禁止远程 CSS 和脚本；这是当前公开行为而非 Obscura 的要求。用户目标是原样渲染 HTML 及所需资源，Obscura 已能自行加载资源，因此建议合并为 `image` 模式并移除 `imageWithRemote` 和专门的远程图片准备链，`image` 改为直接加载页面需要的资源，不沿用旧过滤路径。
- 该调整改变了现有公开 API 和文档，需同步枚举、调用方、schema、文档和相关测试。现有分片续读依赖远程图片快照以及 cursor 的资源版本，移除该链时需同时处理分片续读行为。当前仅为方案建议，未实施。

## 提交用户确认的修复方向

- 使用自维护且可供发行构建取得的 Obscura 0.2.6 Rust 源码，包含已实测的系统字体文件映射改动；不增加引擎选择和单独安装的浏览器。
- 从原始邮件做 MIME、字符集解码后直接传入 HTML；将 CID 对应的 MIME 内容以资源提供方式交给页面，不继续当前 CID 字符串全局替换与图片类型白名单。删除 litehtml、HTML 清理/重写、强制字体/禁动画 CSS 和 imageWithRemote 专用预下载/过滤。页面正常加载其资源，遵循用户要求不自行增加控制。
- API 保留 text 与 image，image 允许加载页面资源，删除 imageWithRemote 及旧的兼容逻辑，调整公开 schema/文档与调用方。保留 PNG 分片/续读能力；以同一次页面渲染得到的分片结果支持续读，避免续片时重新获取远程资源导致页面不一致；具体资源与截图接入须实现验证。
- 复用现有一次性 agentic-gpt worker，Obscura page/runtime 同线程创建、使用、销毁；请求结束、取消或 Agent 退出时正确回收。去掉旧渲染路径中与直接渲染相冲突的 512 MiB 地址空间、20 秒 CPU、30 秒 worker 超时等限制，不新增限制；MCP 现有 8 MiB 消息上限通过分片满足。邮件存储层的原始邮件 32 MiB 上限是既有独立行为，此次不擅自改变。
- 修正四种 GNU/musl 构建链所需的 V8 构建和 snapshot 问题，维护 Obscura/V8/内置字体等许可证随发行包；远端通过 Agentic 实际入口验证两封原始邮件并展示图片，核对效果、资源占用和实际发版链。方案待用户确认，尚未实现。

- 用户询问 CID 内嵌资源是否已验证：检查两封原始邮件后，它们均无 HTML `cid:` 引用，也无 `Content-ID` MIME 部件。此前测试只把提取的 HTML 和远程图片交给 Obscura，没有测试 MIME 内嵌资源。上面不改写 HTML 的 CID 资源映射是待验证的设计方向，不是已经验证可用的 Obscura 接口。
- 补查 Obscura v0.2.6 源码：渲染层有 `RenderResourceLoader`，但 URL 解析与页面资源加载路径有 scheme 处理；不能据此认为直接注入 loader 就已支持 `cid:`。CID 需要用真实含 Content-ID 的邮件检查实际接入链及截图，再判断是否需修改 Obscura。

## CID 内嵌资源远端实测

- 用户要求实测，原先两封邮件没有 CID；从连接的 Gmail 中找到真实 `Delivery Status Notification (Failure)` 原始邮件，HTML 有 1 个 `cid:` 引用，MIME 有 1 个对应 144×144 PNG 内嵌部件。原邮件 HTML 没有改写；测试在 Laptop 使用此前文件映射版 Obscura 0.2.6 的临时源码。
- 先在不提供 CID 资源时截图，图标位置显示替代文字；试验版调整 Obscura 对 `cid:` 的资源 URL 识别，并在页面加载完成后将原 MIME 图片数据按 CID 注入页面资源缓存，再截图。图标正常出现；两图差异区域为 (33,68)-(114,149)。全部内容均来自同一封实际邮件，不修改 HTML。
- 该原型是直接在 Obscura Rust `Page` 上手动注入对应 MIME 图片，不是 Obscura 原版开箱即用，不是已经完成 Agentic 的 MIME 解析与资源接入。仅验证该真实邮件中的 PNG 图像呈现；资源注入时机、其他 MIME 资源和页面脚本中图片状态还需在正式集成中落实。
- 图片位于既有图片目录：`cid-real-without.png` 为未提供内嵌资源的对照，`cid-real-with-mapping.png` 为 CID 资源映射后的截图，`cid-real-original-image.png` 为原邮件 MIME 内嵌图片。
