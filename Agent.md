# Agent.md

本文件为 AI 编程助手在本仓库中工作时提供指引。

详细的架构设计见 `Readme.md`，代码 API 文档在各 crate 的 `lib.rs` 中。

## 构建与测试

**工具链**: nightly-2026-04-25，目标 `riscv64imac-unknown-none-elf`，需 `rust-src`、`llvm-tools`、`rustfmt`、`clippy` 组件。

```bash
make test                # 全部集成测试
make test.smoke          # 单个测试
make test.preempt_spawn  # 单个测试
make demo.async_fn       # 单个 demo

cargo check --target riscv64imac-unknown-none-elf -p test --features qemu-virt  # 编译检查
```

测试以 QEMU 二进制运行，输出 `PASS`/`FAIL`，失败日志在 `/tmp/rt-async-<name>.log`。

## 调试

- `log::info!()`/`log::error!()` 输出到 QEMU UART，格式 `[LEVEL] file: message`
- 使用前需调用 `platform::init()` 初始化 logger
- 日志是阻塞的（逐字节写 UART 寄存器），不要在中断上下文高频打印

## Crate 依赖关系

```
executor-macro (#[task], #[main], #[interrupt])
       ↓
executor ←→ platform (Chip, TimerChip via extern-trait)
   ↓         ↓
futures ← timer (ISR 截止时间队列)
   ↓
apps/test, apps/demo
```

## Unsafe 规范

本仓库大量使用 unsafe，需严格遵守以下规则：

- **所有共享状态的读写必须在 `critical_section::with()` 中完成**。即使在 ISR 中 MIE=0 时 critical section 功能上冗余，也必须使用——保证代码在不依赖中断状态的上下文中也可安全调用。
- **手动 `Sync` impl 必须说明安全性依据**，通常要求 `T: Send`。参考 `util.rs` 中的模式。
- **裸指针只在 critical section 内使用**，不跨临界区传递。
- **ISR 回调（timer 模块）**：回调在中断上下文中执行，不可阻塞、不可 panic、不可操作互斥锁。
- 每个 `unsafe` 块上方必须用注释说明为何安全。

## 常见任务

### 添加新测试

1. 在 `apps/test/src/bin/` 下创建新文件，声明 `#![no_std]` `#![no_main]`
2. 在 `apps/test/Cargo.toml` 添加：
   ```toml
   [[bin]]
   name = "test_name"
   required-features = ["qemu-virt"]
   ```
3. 测试模板（`#[executor::main]` 自动生成 `platform::init()`、spawner 初始化、`platform::start()` 和 `MachineSoft` ISR，不要手动写这些）：
   ```rust
   #![no_std]
   #![no_main]

   #[cfg(feature = "qemu-virt")]
   extern crate qemu_virt;

   #[executor::task]
   async fn my_task() {
       unsafe { test::record("event") };
       test::assert_log(&["event"]);
       platform::ChipImpl::shutdown();
   }

   #[executor::main]
   fn main(spawner: Pin<&'static Spawner<4>>) {
       spawner.spawn(Priority::new(0), my_task().unwrap());
   }
   ```
4. 运行 `make test.test_name` 验证

### 添加新的异步原语

1. 在 `modules/futures/src/` 下创建新模块，在 `lib.rs` 中 `pub mod`
2. 内部状态用 `critical_section::Mutex<UnsafeCell<T>>` 保护
3. 实现 `Future` trait，在 `Poll::Pending` 时存储 waker
4. waker 存储用 `*const Option<Waker>` + critical section 访问
5. 实现 `Drop` 清理状态并唤醒下一个等待者
6. 在 `apps/test/src/bin/` 添加集成测试
7. 参考 `mutex.rs` 作为完整示例

### 添加新芯片/平台

1. 在 `modules/platform/chips/` 下创建新 crate
2. 用 `extern_trait` 的 `#[extern_trait_impl]` 为 `Chip` 和 `TimerChip` 提供实现
3. 在需要使用的 app 的 `Cargo.toml` 中添加依赖和 feature gate
4. 参考 `qemu-virt/` 或 `std-chip/`

## 提交前流程

每次用户要求提交时，按以下顺序执行：

1. **Subagent 审阅 diff**：对待提交的 diff 进行审阅。审阅内容包括：
   - 变更是否与用户描述的意图一致
   - 是否有遗漏的文件或多余的变更
   - commit message 是否准确反映变更内容
2. **审阅通过后**，启动 **Subagent 检查文档是否需要同步更新**（跳过周报和技术报告）。检查仓库中的文档（如 `Readme.md`、`Agent.md` 等）是否因本次变更而需要修改，将需要更新的文档清单反馈给用户。

## 约定

- 所有 commit 遵循约定式提交（Conventional Commits）：`<type>: <description>`，type 包括 `feat`、`fix`、`docs`、`refactor`、`test`、`chore` 等，描述使用中文。
- 代码注释和 commit message 使用中文，API 文档使用英文。
- Edition 2024，resolver 3。
- 除 `std-chip` 外全部 `#![no_std]`，禁止动态内存分配。
- Feature flag：`qemu-virt` 控制硬件相关代码，`std` 控制宿主可测试代码。
- `#[task]` 宏约束：必须是 `async fn`，返回 `()`，不支持泛型和 `self` 参数。
- `SpawnToken` 必须消费（否则 panic），不可丢弃。
- 测试为集成测试，位于 `apps/test/src/bin/*.rs`，每个是独立 QEMU 二进制。
