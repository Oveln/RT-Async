# AGENTS.md

本文件为 AI 编程助手（ZCode 等）在本仓库工作时提供指引。详细的架构设计见
`README.md`，代码 API 文档在各 crate 的 `lib.rs` 顶部模块注释中。

> 文件名沿用历史 `Agent.md`（git 已追踪），内容随架构演进持续更新。当前版本
> 反映 **driver model（Board / Driver / Slot / DeviceRegistry）** 之后的架构。

---

## 1. 仓库定位

**rt-async** 是基于 Rust 的 `#![no_std]` async RTOS 内核：优先级抢占 + 同优先级
协程协作的混合调度，所有 executor 共享一个系统栈（零额外栈切换开销）。本仓库是
`rt-async-amp`（异构多核双内核 AMP 系统）项目的**子模块**，仅含内核 + 平台抽象 +
测试 demo；板级驱动（K3 / QEMU-virt）与 AMP 集成在主仓库 `rt-async-amp` 中。

- 集成分支：`main`
- 远端：<https://github.com/Oveln/rt-async>
- 上游仓库（主仓）：`rt-async-amp`，以 git submodule 引用本仓

---

## 2. 构建与测试

**工具链**：`nightly-2026-04-25`（见 `rust-toolchain.toml`），目标三元组
`riscv64imac-unknown-none-elf`，组件 `rust-src` / `llvm-tools` / `rustfmt` / `clippy`。

```bash
# 内核 + 平台编译检查（本仓是独立 workspace）
cargo check --target riscv64imac-unknown-none-elf -p platform
cargo check --target riscv64imac-unknown-none-elf -p executor

# demo / test（QEMU 二进制，输出 PASS/FAIL）
make test                # 全部集成测试
make test.smoke          # 单个测试
make test.preempt_spawn  # 单个测试
make demo.async_fn       # 单个 demo
```

测试以 QEMU 二进制运行，输出 `PASS`/`FAIL`，失败日志在 `/tmp/rt-async-<name>.log`。

> **注意**：本仓的 `.cargo/config.toml` 定义了 `runner`，而主仓的
> `.cargo/config.toml` 也定义了同名 `runner`（数组形式）。当主仓作为父目录加载
> 本仓时，cargo 会因两者 `runner` 类型（string vs array）不同而合并失败。**始终在
> 本仓目录内单独执行 cargo / make 命令**，不要从主仓根目录跨子模块构建。

---

## 3. 架构（当前 driver model）

```
executor-macro   (#[task] / #[main] / #[interrupt] 过程宏)
       │
executor ◄──────► platform          (内核调度 ↔ 平台抽象)
   │                 │
futures            driver model      (Driver / Slot / DeviceRegistry / bus)
   │                 │
timer (ISR 截止时间队列)   device.rs    (Driver + 功能 trait 契约层)
                     │
                   板级 crate（在主仓：chip-k3-rt24 / chip-qemu-virt-rt）
```

### driver model 核心概念（`modules/platform/src/`）

| 文件 | 职责 |
|------|------|
| `device.rs` | 契约层：`Driver` trait（`compatible()` + `probe()`）+ 功能 trait（`Serial`/`Timer`/`Ipi`/`Reset`/`InterruptController`/`PinController`/`I2cBus`）。**改驱动接口只动这里。** |
| `driver.rs` | 中枢：全局 `Slot<T>` 槽位（console/timer/ipi/reset/intc/pinctrl）+ `DeviceRegistry`（多实例）+ `boot()` DFS 遍历设备树按 compatible 实例化 driver。 |
| `bus.rs` | bus 抽象（i2c/spi controller → child device 寻址），depth-aware boot。 |
| `dtb.rs` | FDT 解析层 + DTB handoff。 |
| `irq.rs` | 中断分发。 |
| `logger.rs` | `log` facade → console；boot 早期 console 未就绪时静默丢弃（不 panic）。 |

**关键设计**：
- **`Driver` 是"可被 DT 探测"的入口，功能 trait 是"这个设备能做什么"**。一个设备可
  同时实现多个功能 trait（如 CLINT 同时 impl `Timer` + `Ipi`）。
- driver 实例是**零大小单例**，`probe(&self)` 内部用全局 `AtomicUsize` 落地 MMIO
  基址，不依赖 `self` 携带数据。
- 注入采用**直写式**：driver 的 `probe` 直接调用 `CONSOLE::set` / `TIMER::set` 等
  公开槽位；板级 driver 列表由板级 glue 经 `DRIVERS::set` 注入（避免 platform
  反向依赖 driver crate）。
- `boot()` 遍历 DT 时，对每个节点先 `try_pinctrl().apply(node)` 再 probe——保证外
  设引脚在驱动 probe 前就绪（DFS 先序保证 pinctrl controller 节点先 probe）。

### 添加新功能 trait（如未来的 SpiBus）

1. 在 `device.rs` 定义 `pub trait XxxBus: Send + Sync { ... }`，含 `&self` 方法。
2. 在 `driver.rs` 加 `pub static XXX: Slot<&'static dyn XxxBus> = Slot::new();` +
   `pub fn try_xxx() -> Option<...>`（不 panic 的访问器）。
3. 在 `boot()` 循环中按需插入跨节点的统一处理（如 pinctrl 的 `apply`）。
4. 板级 driver 在 `probe` 中 `XXX.set(&INSTANCE)` 注入。

---

## 4. Unsafe 规范

本仓库大量使用 unsafe，需严格遵守：

- **所有共享状态的读写必须在 `critical_section::with()` 中完成**。即使在 ISR 中
  MIE=0 时 critical section 功能上冗余，也必须使用——保证代码在不依赖中断状态的
  上下文中也可安全调用。
- **手动 `Sync` impl 必须说明安全性依据**（参考 `driver.rs` 的 `Slot`：单 hart 串
  行 set，之后只读）。通常要求 `T: Send`。
- **裸指针只在 critical section 内使用**，不跨临界区传递。
- **ISR 回调（timer 模块）**：回调在中断上下文中执行，不可阻塞、不可 panic、不可
  操作互斥锁。
- 每个 `unsafe` 块上方必须用注释说明为何安全。

---

## 5. 提交前流程

每次用户要求提交时，按以下顺序执行：

1. **审阅 diff**：对本次 diff 审阅——变更是否与用户意图一致、是否有遗漏/多余文
   件、commit message 是否准确。
2. **验证编译**：改动了哪个 crate 就 `cargo check --target riscv64imac-unknown-none-elf -p <crate>` 验证。改了 platform 必须验证 platform + 受影响的板级 crate。
3. **检查文档是否需要同步更新**（跳过周报和技术报告）：检查 `README.md`、本文件
   `AGENTS.md` 等是否因本次变更而需要修改，将需要更新的文档清单反馈给用户。
4. 审阅通过后再提交。

---

## 6. Git 工作流

### 分支与集成分支

- **集成分支：`main`**（本仓）。feature 分支从 `main` 切出，完成后 `--no-ff` 合
  并回 `main`。
- 分支命名：`feat/<topic>`（如 `feat/pinctrl-k3`、`feat/driver-registry-refactor`）。
  修复类可用 `fix/<topic>`，重构用 `refactor/<topic>`，但历史以 `feat/` 为主。

### 子模块协调（重要）

本仓是主仓 `rt-async-amp` 的子模块。当一次改动**同时涉及本仓和主仓**（典型：改了
platform 框架 + 板级 driver）时，采用**双仓分支 + 子模块指针 bump** 的流程：

1. 在本仓 `feat/<topic>` 分支开发并提交。
2. 在主仓 `feat/<topic>` 分支开发板级部分，期间把子模块指针 bump 到本仓分支最新
   commit 并提交（`git add rt-async && git commit -m "submodule(rt-async): bump ..."`）。
3. 合并时**先合并子仓**（本仓 `feat/<topic>` → `main`，`--no-ff`），再在主仓 feature
   分支把子模块指针 bump 到本仓 `main` 的 merge commit，提交后再 `--no-ff` 合并主
   feature 分支到 `master`。详见主仓 `AGENTS.md` 的"双仓合并流程"。

### Commit 约定

- 约定式提交（Conventional Commits）：`<type>(<scope>): <描述>`。
  - type：`feat` / `fix` / `docs` / `refactor` / `test` / `chore` / `build`
  - scope：crate 或模块名（`platform` / `executor` / `driver` / `futures` / `timer` / `build`…）
  - 描述**用中文**。
- 例：`feat(driver): Step 4 - bus 抽象 + depth-aware boot（为 i2c/spi 铺路）`
- **代码注释和 commit message 用中文，类型/变量/API 标识符用英文。**

---

## 7. 约定

- Edition 2024，resolver 3。
- 除 `std-chip` 外全部 `#![no_std]`，**禁止动态内存分配**（无 alloc）。所有状态用
  `static` + `AtomicUsize` / `Slot<T>` / `DeviceRegistry<T, N>` 承载。
- Feature flag：`riscv64` 控制硬件相关代码（arch 专有寄存器操作）；`std` 控制宿主
  可测试代码（`std-chip`）。
- `#[task]` 宏约束：必须是 `async fn`，返回 `()`，不支持泛型和 `self` 参数。
- `SpawnToken` 必须消费（否则 panic），不可丢弃。
- 日志：`log::info!()`/`log::error!()` 输出到 console UART。日志是阻塞的（逐字节写
  UART），**不要在中断上下文高频打印**。
- 测试为集成测试，位于 `apps/test/src/bin/*.rs`，每个是独立 QEMU 二进制，输出
  `PASS`/`FAIL`。

---

## 8. 常见任务

### 添加新测试

1. 在 `apps/test/src/bin/` 下创建新文件，声明 `#![no_std]` `#![no_main]`。
2. 在 `apps/test/Cargo.toml` 添加 `[[bin]]`（`required-features = ["qemu-virt"]`）。
3. 用 `#[executor::main]`（自动生成 `platform::init()`、spawner、`platform::start()`
   和 ISR，不要手写这些）+ `#[executor::task]` async 任务。
4. `make test.<name>` 验证。

### 添加新的异步原语

1. 在 `modules/futures/src/` 下创建新模块，`lib.rs` 中 `pub mod`。
2. 内部状态用 `critical_section::Mutex<UnsafeCell<T>>` 保护。
3. 实现 `Future`，`Poll::Pending` 时存储 waker。
4. 实现 `Drop` 清理状态并唤醒下一个等待者。
5. 在 `apps/test/src/bin/` 添加集成测试。参考 `mutex.rs` 作为完整示例。

### 添加新芯片/平台支持（板级 crate 在主仓）

板级 crate（`chip-k3-rt24` / `chip-qemu-virt-rt`）位于主仓 `modules/`。流程见主仓
`AGENTS.md` 的"添加新芯片支持"。本仓的 `platform` 提供契约层，不应反向依赖具体板
级 crate。
