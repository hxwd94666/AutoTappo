# AGENTS.md · AutoTappo 延续开发架构指南

> 面向后续人类开发者与 AI Agent。  
> 当前版本：0.1.0  
> 本文以当前代码为准；历史逆向记录已压缩为必要约束，不再保留过期实现细节。

---

## 1. 项目定位

AutoTappo 是 [To0Ru/AutoTappo](https://github.com/To0Ru/AutoTappo) 的独立延续开发版本。

- 保留上游的刷数、开箱、托盘和自启能力；
- 继续维护后续新增的位置排列、一键加数、单实例和高级开箱失败保护；
- 不代表上游官方，不声称与上游团队存在隶属关系；
- 新仓库发布时必须保留上游链接和延续开发说明。

---

## 2. 目录结构

    src-tauri/src/
    ├── main.rs                 入口：DPI、单实例、session、调用 app::run
    ├── app.rs                  Tauri Builder、worker 装配、配置监听、托盘、窗口生命周期
    ├── state.rs                Inner / AppState
    ├── commands/
    │   ├── mod.rs
    │   ├── system.rs           状态、窗口、拖拽
    │   ├── runtime.rs          启停、速度、热键、模式、一键加数
    │   ├── chest.rs            自动开箱、高级开箱
    │   ├── layout.rs           位置排列
    │   └── autostart.rs        开机自启
    ├── config/
    │   └── settings.rs         settings.json 读写
    ├── platform/
    │   ├── single_instance.rs  Windows 命名互斥
    │   ├── autostart.rs        任务计划
    │   ├── session.rs          锁屏/息屏检测
    │   ├── tray.rs             托盘
    │   └── window.rs           主窗口生命周期
    └── features/
        ├── clicker.rs          键盘连点 worker
        ├── silent.rs           静默计数 worker
        ├── advanced.rs         高级开箱 CDP worker
        ├── arrange.rs          位置排列注入 worker
        ├── injector.rs         WuwaTappo 启动、DLL 注入
        ├── cdp.rs              CDP 客户端
        ├── uia.rs              UIA 探测
        └── chest/              自动开箱引擎
            ├── mod.rs
            ├── geometry.rs
            ├── pixel.rs
            └── state.rs

    web/
    ├── index.html
    ├── ui/
    │   ├── app.js
    │   ├── style.css
    │   └── logo.png
    ├── injected/
    │   ├── room-layout-core.js
    │   └── room-layout.js
    └── tests/
        └── room-layout-core.test.js

    src-tauri/native/autotappo_hook/   创建进程 hook，给 WebView2 追加调试端口

---

## 3. 依赖方向

    main -> app -> commands
    app -> state -> features
    app -> platform
    features -> platform（仅 session/window 等明确依赖）
    config -> features（恢复配置时引用控制器）
    commands -> state / features / config / platform

约束：

- platform 不得反向依赖 commands；
- features 不得依赖 app；
- 算法模块不得直接依赖 Tauri 窗口对象；
- web/ui 不得引用 web/injected；
- web/injected 不引用 UI 文件。

---

## 4. 运行时装配顺序

app.rs 的 setup 顺序：

1. 创建 SilentController；
2. 用 silent.running 创建 ClickerController；
3. 启动 silent、clicker worker；
4. 创建并启动 ChestController；
5. 创建 AdvancedController，并启动高级 worker；
6. 启动 arrange worker；
7. 加载 settings 并 apply；
8. 创建 Inner/AppState，交给 Tauri；
9. 启动配置落盘线程；
10. 构建托盘；
11. 迁移自启参数；
12. 非 --autostart 启动时创建主窗口。

不要改变这个顺序，除非同时验证：

- 设置能正确恢复；
- 自动开箱和高级开箱开关状态正确；
- 托盘与窗口生命周期正常。

---

## 5. 不可破坏的不变量

### 数据格式

- settings.json 路径：%APPDATA%\AutoTappo\settings.json
- 位置排列 key：autotappo-room-layout-v8:<account>:<room>
- 位置存档版本：v8，仅存模式槽位、锚点和换行数量，不绑定好友 ID
- 基础好友槽位：19 个；槽位满后追加溢出槽
- 排列模式：h-left / h-right / v-up / v-down / overlap
- 一键加数范围：1..=1_000_000

### 运行行为

- 单实例名：Local\AutoTappo-single-instance
- 静默模式默认开启；
- 静默模式不发送键盘事件；
- 静默预算：30 分钟 2000~2150，单次 40~90；
- 自动开箱临近就绪每 5 秒复查，最长 60 秒；
- 高级开箱每次启用只尝试一次注入/重启；
- 高级开箱失败显示原因并停止自动重启；
- 本项目没有自动更新；
- 前端没有会话清零入口。

### 安全红线

- 不伪造 HTTP/WebSocket 请求；
- 不移动真实鼠标；
- 不发送鼠标点击；
- 高级开箱只通过官方 WebView 和官方 openChest；
- 不自动下载或执行未知可执行文件。

---

## 6. 功能模块

### 6.1 刷数

clicker.rs：

- 键盘模式：F21~F24，随机按住/间隔，礼貌避让；
- 静默模式：不发送按键，只由 silent.rs 写本地缓存；
- 锁屏/息屏：session::should_pause() 后完全停手；
- F5 热键在键盘 worker 内轮询 GetAsyncKeyState。

silent.rs：

- 读取/原子写入官方 key_press 缓存；
- 支持一键加数队列；
- 失败时保留队列并显示错误；
- 官方同步后清零状态。

### 6.2 自动开箱

chest 模块：

- 优先 UIA 读取 reward-btn / reward-countdown / 矩形 / 剩余秒数；
- UIA 不可用时回退到窗口区域 + 像素；
- 就绪时 PostMessage 点击官方输入窗口；
- 点击后 350~400ms 复查；
- 识别失败进入退避，不伪造请求。

### 6.3 高级开箱

advanced.rs + injector.rs + cdp.rs：

- 依赖自动开箱开启；
- 检测 CDP 后启动/重启 WuwaTappo 并注入 autotappo_hook.dll；
- 通过 Debugger 捕获官方 openChest；
- 每次启用只允许一次注入/重启；
- 失败状态显示在设置行，不再自动无限重启；
- CDP 捕获失败先重试 3 次，失败时清理断点并 resume/disable Debugger；
- WuwaTappo 路径必须精确匹配 WuwaTappo.exe，防止误启动其他鸣潮工具。

### 6.4 位置排列

arrange.rs + web/injected：

- CDP 注入 room-layout-core.js 和 room-layout.js；
- 只修改官方 vnode / 位置缓存，不移动真实鼠标；
- 保存完整壳和可见矩形；
- 横左/横右/竖上/竖下使用 v8 槽位：自己为锚点，19 个基础好友槽，满后追加；
- 槽位不绑定好友 ID，新好友填最低空槽，离开时最高占用槽补位；
- 每行/列数量包含自己；0 或空为到顶换行；
- 换行方向：横排可选向上/向下，竖排可选向左/向右；
- 新好友使用同行/列现有好友的平均间距；
- 自己拖动不带动槽位；切模式时按当前自己位置平移槽位；
- 手动布局单键：第一次进入编辑，第二次保存；保存时更新好友当前占用槽位；
- 位置排列默认收起，不记忆展开状态；重置排列已移除；
- 重叠模式把所有好友猫放在自己壳位置、z-index 5，跟随自己拖动，不写入槽位；
- 新成员/返回成员有短暂隐藏保护；
- 观察器改动风险较高，修改后必须实测新进、离线返回、F6 手动布局、横竖/重叠切换；
- 默认单实例，避免两个 worker 互相覆盖模式。

### 6.5 Web UI

web/ui/app.js：

- 无构建器，直接由 index.html 加载；
- 负责状态轮询、设置、热键录制、位置排列 UI、一键加数；
- 不得引用 web/injected 脚本；
- 所有 Tauri command 名称必须与后端 handler 一致。

---

## 7. Tauri 命令

当前命令：

    get_status
    toggle_running
    set_speed
    set_hotkey
    set_mode
    add_clicks
    minimize_window
    close_window
    start_drag
    get_chest_status
    set_auto_chest
    get_advanced_status
    set_advanced_enabled
    set_advanced_flags
    get_auto_arrange
    set_auto_arrange
    get_layout_mode
    set_layout_mode
    get_layout_wrap
    set_layout_wrap
    get_layout_wrap_dir
    set_layout_wrap_dir
    get_layout_edit
    get_layout_edit_hotkey
    set_layout_edit_hotkey
    get_autostart
    set_autostart

修改命令时同步：

- commands 模块实现；
- app.rs generate_handler 路径；
- web/ui/app.js invoke 调用；
- 测试与文档。

---

## 8. 测试与构建

Rust：

    cd src-tauri
    cargo test
    cargo check

Web：

    node --check web/ui/app.js
    node --check web/injected/room-layout.js
    node --test web/tests/room-layout-core.test.js

原生 DLL：

    cargo build --release --manifest-path src-tauri/native/autotappo_hook/Cargo.toml

构建：

    cd src-tauri
    npx @tauri-apps/cli@^2 build

验收：

- 35 个 Rust 测试；
- 21 个 Node 布局测试；
- 命令名完全对应；
- 冷启动窗口正常；
- 托盘退出/重开正常；
- 无旧路径引用。

---

## 9. 已知限制

- 不内置自动更新；
- 高级开箱依赖管理员权限、WebView2、DLL 注入，可能被安全软件拦截；
- 官方改版可能导致 UIA/DOM/CDP 捕获失效；
- 自动开箱倒计时按官方秒级 DOM 读取，可能有约 1 秒误差；
- 位置排列只影响本地客户端；
- 新成员/返回成员的短暂隐藏是视觉保护，不是核心排列逻辑；
- 上游仓库当前没有明确 LICENSE 文件，发布前必须确认授权。

---

## 10. 工作准则

### 修改前

- 先读本文件和 docs/REFACTOR_PLAN.md；
- 确认要改的模块和依赖方向；
- 不把平台代码放进 features；
- 不把命令实现散落回 main.rs。

### 修改后

- 至少运行对应测试；
- 涉及 Tauri command 时核对前端 invoke；
- 涉及位置排列时实测 F6 手动布局、新成员、返回成员；
- 涉及高级开箱时实测失败状态和单次重启；
- 提交信息写清改动和原因。

### Git 边界

- 本地 commit 可直接做；
- push / tag / Release 必须先获得用户明确同意；
- 不提交 research、target、临时脚本和构建产物。

### 文档

- README 只面向用户；
- AGENTS 面向开发者；
- 新功能必须同步修改命令表、目录结构和测试说明；
- 不保留已移除功能的文档承诺。

---

## 11. 延续开发声明

本项目是上游 AutoTappo 的延续开发，不是官方版本。

任何发布、宣传和文档都应明确：

- 基于 To0Ru/AutoTappo；
- 独立维护；
- 上游链接；
- 与上游的功能差异；
- 不代表上游团队。
