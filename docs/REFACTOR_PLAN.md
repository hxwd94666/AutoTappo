# AutoTappo 延续开发重构方案

> 状态：已完成  
> 目标版本：0.1.0  
> 原则：只调整工程结构与文档，不改变现有用户可见功能和运行时行为。

## 1. 项目定位

本项目是 To0Ru/AutoTappo 的延续开发版本。

- 保留上游能力：静默刷数、键盘连点、自动开箱、高级开箱、托盘、开机自启；
- 保留后续能力：位置排列、一键加数、单实例、注入失败保护；
- 本项目为独立延续仓库，不代表上游官方，也不声称与上游团队存在隶属关系；
- README 必须明确写出延续开发、独立维护、上游链接和当前差异。

本次重构不改变程序名称、包名、命令名、localStorage key、配置字段和存档格式。

## 2. 当前问题

1. main.rs 同时承担入口、状态定义、单实例、窗口生命周期、托盘装配、worker 启动和命令注册；
2. commands.rs 把运行时、窗口、宝箱、布局、自启全部混在一个文件；
3. 平台模块和业务模块混在 src 根目录；
4. Web 资源混放，UI、注入脚本和测试没有目录边界；
5. README 保留已移除的自动更新等旧承诺；
6. AGENTS 过长，历史记录淹没当前架构和约束。

## 3. 目标 Rust 结构

    src-tauri/src/
    ├── main.rs                     # 最小入口
    ├── app.rs                      # Builder、setup、worker 装配、托盘、配置监听
    ├── state.rs                    # Inner / AppState
    ├── commands/
    │   ├── mod.rs                  # re-export，保持命令名
    │   ├── system.rs
    │   ├── runtime.rs
    │   ├── chest.rs
    │   ├── layout.rs
    │   └── autostart.rs
    ├── config/
    │   ├── mod.rs
    │   └── settings.rs
    ├── platform/
    │   ├── mod.rs
    │   ├── single_instance.rs
    │   ├── autostart.rs
    │   ├── session.rs
    │   ├── tray.rs
    │   └── window.rs
    └── features/
        ├── mod.rs
        ├── clicker.rs
        ├── silent.rs
        ├── advanced.rs
        ├── arrange.rs
        ├── injector.rs
        ├── cdp.rs
        ├── uia.rs
        └── chest/
            ├── mod.rs
            ├── geometry.rs
            ├── pixel.rs
            └── state.rs

### 文件迁移表

| 原路径 | 新路径 |
|---|---|
| main.rs 中的 Inner/AppState | state.rs |
| main.rs 中的 Builder/setup | app.rs |
| main.rs 中的单实例代码 | platform/single_instance.rs |
| autostart.rs | platform/autostart.rs |
| session.rs | platform/session.rs |
| tray.rs | platform/tray.rs |
| window.rs | platform/window.rs |
| settings.rs | config/settings.rs |
| clicker.rs | features/clicker.rs |
| silent.rs | features/silent.rs |
| advanced.rs | features/advanced.rs |
| arrange.rs | features/arrange.rs |
| injector.rs | features/injector.rs |
| cdp.rs | features/cdp.rs |
| uia.rs | features/uia.rs |
| chest.rs | features/chest/mod.rs |
| chest/* | features/chest/* |
| commands.rs | commands/*.rs |

### 依赖方向

    main -> app -> commands
    app -> state -> features
    app -> platform
    features -> platform（仅 session/window 等明确依赖）
    config -> features（恢复配置时引用控制器）
    commands -> state / features / config / platform

禁止：platform 反向依赖 commands；features 依赖 app；算法模块依赖 Tauri 窗口对象；Web 注入脚本引用 UI。

## 4. 目标 Web 结构

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

- index.html 引用 ui/style.css、ui/app.js；
- Rust include_str! 和测试路径同步更新；
- 本次不拆 ui/app.js，避免无构建器的加载顺序风险；
- 注入脚本保持独立 IIFE，不引用 UI 文件；
- README 图片改为 web/ui/logo.png。

## 5. 行为保持清单

重构期间必须保持：

- Tauri command 名称、参数、返回结构不变；
- 设置文件路径和字段不变；
- autotappo-room-layout-v3:* key 不变；
- 位置排列 v3 存档格式不变；
- 单实例名称不变；
- 高级开箱失败策略不变；
- 静默预算 2000~2150/30 分钟不变；
- 一键加数范围 1~1,000,000 不变；
- 现有快捷键和 UI 文案不变（除结构路径）；
- 不新增网络请求，不改变开箱交互方式。

## 6. 分阶段执行

### Phase 0：基线

- 记录 git status、测试数量、构建包；
- 工作区干净则打基线提交。

### Phase 1：Web 目录整理

- 创建 web/ui、web/injected、web/tests；
- git mv 资源；
- 更新 index.html、Rust include_str!、测试路径、README 图片；
- 运行 node --test、node --check。

### Phase 2：Rust 平台层与业务层

- 创建 platform、features；
- git mv 模块；
- 更新 mod 与 use 路径；
- 运行 cargo check、cargo test。

### Phase 3：状态与入口拆分

- 新建 state.rs、app.rs；
- 迁移 Inner、AppState、Builder、setup、配置监听、托盘装配；
- main.rs 只保留入口、DPI、单实例、session；
- 运行测试和启动冒烟。

### Phase 4：命令拆分

- 按域拆分 commands.rs；
- commands/mod.rs re-export，保持 generate_handler! 名称不变；
- 运行测试，核对前端 invoke 名称。

### Phase 5：文档重写

- 完全重写 README：延续关系、上游致谢、功能、安装、已知限制、构建、许可证说明。
- 完全重写 AGENTS：目录、模块职责、不变量、测试、边界、上游差异。
- 不虚构许可证，不虚构官方关系。

### Phase 6：验收

- cargo test
- node --test
- cargo check
- 构建 0.1.0 便携版 + NSIS
- 启动冒烟
- 比对 command 列表
- 检查旧路径引用
- 提交最终重构

## 7. 测试矩阵

| 层级 | 命令 | 通过标准 |
|---|---|---|
| Rust | cd src-tauri && cargo test | 35 通过 |
| Rust 编译 | cargo check | 无 warning/error |
| 布局核心 | node --test web/tests/room-layout-core.test.js | 12 通过 |
| JS 语法 | node --check web/ui/app.js | 通过 |
| 命令一致性 | 对比 app.js invoke 与 handler | 完全对应 |
| 启动 | 运行便携版 | 窗口、托盘正常 |
| 构建 | npx @tauri-apps/cli build | 生成 0.1.0 安装包 |

## 8. 风险与回滚

风险：

- 模块路径迁移导致 include_str!/测试路径失效；
- commands re-export 与 Tauri handler 名字不一致；
- app.rs 拆分导致启动顺序改变；
- Web 路径变化导致白屏或注入 404；
- import 修改引入编译错误。

控制：

- 每阶段单独提交，可单独 revert；
- 每阶段至少编译/测试一次；
- 不修改算法实现；
- 保留命令名、配置字段、localStorage key；
- 发现行为变化立即回滚该阶段。

回滚：git revert 对应阶段提交；重构前基线可整体回退。

## 9. README 重写要点

1. 标题与一句话定位；
2. 延续开发声明：基于 To0Ru/AutoTappo、独立维护、上游链接、差异概述；
3. 当前功能；
4. 安装与使用；
5. 安全边界；
6. 已知限制；
7. 本地构建；
8. 目录结构；
9. 致谢与许可证说明，不虚构许可证。

## 10. AGENTS 重写要点

1. 项目定位与延续关系；
2. 当前目录结构；
3. 模块职责与依赖方向；
4. 不可破坏的不变量；
5. 开箱、CDP、UIA 的关键约束；
6. 位置排列存储与时序保护；
7. 单实例与高级开箱失败策略；
8. 测试、构建、发布流程；
9. 已知边界；
10. 禁止事项。

## 11. 完成标准

- 目录结构与目标一致；
- 所有功能行为不变；
- Rust 35 测试通过；
- Node 12 测试通过；
- command 名称完全一致；
- README 明确延续开发关系；
- AGENTS 与当前代码一致；
- 0.1.0 安装包构建成功；
- 无旧路径引用；
- 未 push。

## 12. 执行记录

- Phase 1 Web 分层：ecf0b7a
- Phase 2 Rust features/platform/config：8c9239f
- Phase 3 入口与状态拆分：cc597d8
- Phase 4 commands 拆分：e7c0e64
- 文档重写：829d0c8
- 重构后测试：Rust 35 通过，Node 12 通过，cargo check 通过
- 构建：AutoTappo 0.1.0 便携版 + NSIS 安装包
- 启动冒烟：窗口正常，高级开箱状态行正常
