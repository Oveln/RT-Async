# executor 宏文档

## `#[executor::task]` — 静态任务分配

将 `async fn` 转换为静态分配的任务函数，返回 `SpawnToken`。

### 约束

- 必须作用于 `async fn`
- 返回类型必须为 `()`
- 不支持泛型参数
- 不支持 `self` 参数

### 示例

```rust
#[executor::task]
async fn blink(led: usize) {
    // ...
}

// 展开为：
// fn blink(led: usize) -> Result<SpawnToken<impl Future<Output = ()>>, SpawnError>

spawner.spawn(Priority::new(0), blink(13).unwrap());
```

---

## `#[executor::main]` — 应用程序入口

替代手写的 `__rust_main` 和 `MachineSoft`，用户只需编写任务派发代码。

### 生成内容

宏从 `Spawner<N>` 的 const generic 参数中提取优先级数量 `N`，生成三个顶层项：

#### 1. `static mut __SPAWNER`

```rust
static mut __SPAWNER: MaybeUninit<Spawner<N>> = MaybeUninit::uninit();
```

整个程序生命周期的静态 Spawner 存储，被 `__rust_main`（初始化）和
`MachineSoft`（ISR）共享。

#### 2. `#[unsafe(no_mangle)] __rust_main() -> !`

裸机入口点，由汇编 `_start` 跳转进来。执行顺序：

1. `platform::init()` — 日志/平台初始化
2. 创建 `Spawner::new()`，pin 住，调用 `.init()`
3. 执行**用户的函数体**（即任务 spawn 代码）
4. `platform::start()` — 使能 MSI + 全局中断
5. `loop { platform::idle() }` — WFI 休眠

#### 3. `#[unsafe(no_mangle)] MachineSoft(&mut TrapFrame)`

RISC-V 机器软件中断（MSI）处理程序。清除 MSI 挂起标志后，通过
[`platform::PEND_MARKER`] 区分中断来源：

| `PEND_MARKER` | 来源                         | 行为                                                                 |
|---------------|------------------------------|----------------------------------------------------------------------|
| `true`        | `platform::pend()`（调度器） | 运行优先级抢占调度循环：`try_preempt` → `enable_interrupts` → `run` → `disable_interrupts` → `complete_executor` |
| `false`       | 外部 MSI（硬件/核间中断）    | 调用 `__Inner_MachineSoft`（用户自定义钩子，见 [`#[executor::interrupt]`](#executorinterrupt--中断处理)） |

此设计确保调度器 ISR 不会被意外绕过，同时让用户完全掌控非调度器的 MSI 事件。

### 函数签名要求

- 必须有且仅有一个参数，类型为 `Pin<&'static Spawner<N>>` 或 `Pin<&Spawner<N>>`
- 宏在编译期从 const generic 参数提取 `N`

### 约束

- 不能是 `async fn`
- 不支持泛型参数
- 必须恰好一个 Spawner 参数

### 示例

```rust
#![no_std]
#![no_main]

#[executor::main]
fn main(spawner: core::pin::Pin<&'static executor::spawner::Spawner<4>>) {
    spawner.spawn(Priority::new(0), blink(13).unwrap());
    spawner.spawn(Priority::new(1), periodic().unwrap());
}
```

### 与 `#[executor::interrupt]` 的交互

`MachineSoft` 符号始终由此宏生成，不可在其他地方定义。

要处理**外部** MSI 事件（即非调度器 `pend()` 触发的 MSI），用
`#[executor::interrupt]` 定义名为 `MachineSoft` 的函数：

```rust
#[executor::interrupt]
fn MachineSoft(_tf: &mut TrapFrame) {
    // 此函数仅在外部 MSI 时被调用
    // 宏自动将符号名改写为 `__Inner_MachineSoft`
}
```

---

## `#[executor::interrupt]` — 中断处理

将函数转换为 `#[unsafe(no_mangle)] pub unsafe extern "C"` 中断处理函数，
符号名与函数名一致（匹配 RISC-V 中断陷阱分发表）。

### 符号名映射

| 函数名           | 输出符号              | 说明     |
|------------------|-----------------------|----------|
| `MachineTimer`   | `MachineTimer`        | 直接映射 |
| `MachineExternal`| `MachineExternal`     | 直接映射 |
| `MachineSoft`    | `__Inner_MachineSoft` | **特殊**，见下文 |
| *(其他)*         | *(与函数名相同)*      | 直接映射 |

### `MachineSoft` 特殊处理

`MachineSoft` 符号被 [`#[executor::main]`](#executormain--应用程序入口)
保留给调度器 ISR。当用户用此宏定义名为 `MachineSoft` 的函数时，宏自动将输出
符号改写为 `__Inner_MachineSoft`。

运行时，生成的 `MachineSoft` ISR 检查 [`platform::PEND_MARKER`]：

- **系统 pend**（`PEND_MARKER == true`）→ 运行调度器
- **外部 MSI**（`PEND_MARKER == false`）→ 调用用户的 `__Inner_MachineSoft`

如果用户**没有**定义 `MachineSoft` 函数，链接器为 `__Inner_MachineSoft`
提供弱符号默认值（`DefaultHandler`，即 abort），与其他未处理中断行为一致。

### 签名要求

- 必须有且仅有一个 `&mut TrapFrame` 参数
- 返回 `()`
- 宏自动添加 `#[unsafe(no_mangle)]` 和 `unsafe extern "C"` 包装

### 约束

- 不能是 `async fn`

### 示例

定时器中断：

```rust
#[executor::interrupt]
fn MachineTimer(_tf: &mut TrapFrame) {
    // 清除定时器中断、更新 tick 等
}
```

外部 MSI 处理（仅在非调度器触发的 MSI 时被调用）：

```rust
#[executor::interrupt]
fn MachineSoft(_tf: &mut TrapFrame) {
    // 符号被改写为 `__Inner_MachineSoft`
    // 仅在 MSI 由外部触发（而非 pend()）时运行
}
```

---

## 整体架构

```
                         中断触发
                            │
                            ▼
               ┌─────────────────────────┐
               │   riscv64-rt trap 入口   │
               │   (保存上下文 → mcause)   │
               └────────┬────────────────┘
                        │
          ┌─────────────┼─────────────┐
          ▼             ▼             ▼
    MachineTimer   MachineSoft   MachineExternal
          │             │             │
          │    ┌────────┴────────┐    │
          │    │ clear_pend()    │    │
          │    │                 │    │
          │    │ PEND_MARKER?    │    │
          │    ├─ true ──┐      │    │
          │    │         │      │    │
          │    │   调度器循环    │    │
          │    │  try_preempt   │    │
          │    │  enable_ints   │    │
          │    │  run / complete│    │
          │    │         │      │    │
          │    ├─ false ─┘      │    │
          │    │                 │    │
          │    │ __Inner_Machine │    │
          │    │    Soft()       │    │
          │    │  (用户自定义)    │    │
          │    └─────────────────┘    │
          ▼                           ▼
    用户处理函数                  用户处理函数
```

### PEND_MARKER 机制

[`platform::PEND_MARKER`] 是一个 `AtomicBool` 静态量：

- `platform::pend()` 在触发 MSI **之前**将其设为 `true`
- `MachineSoft` ISR 用 `swap(false, AcqRel)` 原子地读取并清除
  - `true` → 本次 MSI 由调度器触发，执行抢占循环
  - `false` → 本次 MSI 来自外部（硬件/核间），调用用户钩子
