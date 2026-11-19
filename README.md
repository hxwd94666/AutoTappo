<p align="center">
  <img src="web/ui/logo.png" width="88" height="88" alt="AutoTappo">
</p>

<h1 align="center">🐾 AutoTappo</h1>

<p align="center">
  <b>WuwaTappo 后台挂机、自动开箱与房间排列助手</b>
</p>

<p align="center">
  <a href="https://github.com/To0Ru/AutoTappo">基于 To0Ru/AutoTappo 的延续开发</a>
  · Windows 10 / 11
  · 版本 0.1.0
</p>

---

> **延续开发**  
> 本项目是 To0Ru/AutoTappo 的独立延续版本，不是上游官方项目。  
> 保留上游核心能力，并继续维护位置排列、一键加数、单实例和高级开箱失败保护。

---

## ✨ 功能

- **静默刷数（默认）**：只写官方本地计数，不发送键盘事件，不播放打字动画。
- **键盘模式**：可选的真实拟真按键，适合需要桌宠动作反馈的场景。
- **礼貌避让**：打字、切换输入法、鼠标划词时自动暂停。
- **自动开箱**：UIA 读取官方状态，失败时回退像素识别；临近就绪每 5 秒复查，最多 60 秒。
- **高级开箱（可选）**：锁屏或 F9 隐藏时也能领取；失败会显示原因并停止自动重启。
- **位置排列**：
  - 手动 / 自动两种模式，默认手动；
  - 自动固定 19 个好友槽，配置全局共用，不保存槽位；
  - 自动方向：横左、横右、竖上、竖下、重叠；
  - 自动可设置每行/列数量（包含自己）和换行方向；
  - 手动存档按房间保存，槽位按距自己由近到远编号；
  - 手动空槽保留，新好友优先填空槽，没有空槽时新好友保持官方默认位置；
  - 自动离开补位，手动离开不补位。
- **一键加数**：输入 1～1,000,000 次，写入官方本地计数缓存。
- **托盘 / 开机自启**。

---

## 🚀 快速开始

### 使用

1. 启动 WuwaTappo。
2. 启动 AutoTappo。
3. 选择静默或键盘模式。
4. 点击开始，或按 **F5**。
5. 需要自动开箱时，在设置中开启自动开箱。
6. 需要锁屏开箱时，再开启高级开箱，并留意设置行的运行状态。

### 下载

- 安装包：AutoTappo_0.1.0_x64-setup.exe
- 便携版：AutoTappo_0.1.0.exe

首次运行需要管理员权限。高级开箱会结束并重启一次 WuwaTappo，以加载调试端口。

### 日志

标题栏右上角的文件夹按钮可直接打开日志目录：

```text
%LOCALAPPDATA%\AutoTappo\logs
```

高级开箱失败时优先查看当日 `autotappo-YYYY-MM-DD.log`，同目录还有 `hook-<PID>.log`，包含 Hook 安装和 WebView2 命令行信息。

---

## 🧩 位置排列

1. 打开高级开箱。
2. 展开设置底部的 **位置排列**。
3. 选择 **手动** 或 **自动**。
4. 自动模式：选择横左、横右、竖上、竖下或重叠，并设置每行/列数量和换行方向。
5. 手动模式：按 **F6** 第一次进入编辑，拖动好友猫，再按一次保存。
6. 自动模式 F6 禁用；重叠模式只属于自动模式。
7. 手动档为空时不会摆位；有手动档时按房间存档应用。
8. 手动模式下所有自动控件保持可见，但显示禁用光标；切换到自动后立即生效。

排列规则：

- 自己作为起点，不计入好友槽位；
- 第一行/列包含自己；
- 换行后，新行/列仍从自己的起点开始，不跳过自己的行/列；
- 自动槽位和手动槽位都不绑定好友 ID；
- 自动配置全局共用，自动槽位不写入 localStorage；
- 手动存档按房间保存，最多 19 个槽位（含空槽）；
- 位置排列默认收起，不记忆展开状态。

位置只保存在当前客户端，不会修改其他玩家的画面。

---

## 🛡️ 安全边界

- 不伪造 HTTP / WebSocket 请求。
- 不移动真实鼠标，不发送鼠标点击。
- 静默模式只修改官方本地计数缓存。
- 高级开箱只在用户显式开启后注入。
- 不内置自动更新，不自动下载可执行文件。

高级开箱依赖 DLL 注入和 WebView2，可能被杀毒软件拦截。请只从可信来源下载，并按需添加信任。

---

## ⚠️ 已知限制

- 不内置自动更新，需要手动下载新版本。
- 高级开箱失败后会停止自动重试，需要关闭再开启。
- 自动开箱依赖 UIA / 像素识别，官方大版本更新后可能需要适配。
- 手动布局依赖官方 WebView 位置接口；接口不可用时自动槽位仍可用，但手动拖动可能不准确。
- 位置排列只影响当前客户端；新成员和返回成员可能有短暂隐藏保护。
- 上游仓库目前没有明确 LICENSE 文件，正式发布前需确认授权。

---

## 🛠️ 本地构建

需要 Rust、Node.js 18+、Tauri CLI 2.x 和 WebView2。

测试：

    cd src-tauri
    cargo test

    node --test web/tests/room-layout-core.test.js

先构建原生注入 DLL：

    cargo build --release --manifest-path src-tauri/native/autotappo_hook/Cargo.toml

构建应用：

    cd src-tauri
    npx @tauri-apps/cli@^2 build

输出：

    src-tauri/target/release/AutoTappo.exe
    src-tauri/target/release/bundle/nsis/AutoTappo_0.1.0_x64-setup.exe

---

## 📁 结构

- src-tauri/src：Rust 后端，按 commands / config / platform / features 分层。
- web/ui：主界面。
- web/injected：注入到官方房间的排列脚本。
- docs：架构与手动/自动排列设计文档。
- src-tauri/src/logging.rs：运行日志与日志目录。

详细模块职责、不变量和开发规范见 [AGENTS.md](AGENTS.md)。

---

## 🤝 致谢与许可

- 上游项目：[To0Ru/AutoTappo](https://github.com/To0Ru/AutoTappo)
- 官方桌宠：WuwaTappo / 鸣潮

本项目是独立延续开发，不代表库洛游戏或上游官方。  
上游仓库目前没有明确 LICENSE 文件，本仓库不单方面声明新许可证。
