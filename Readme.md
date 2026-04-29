# RT-Async

一个基于 Rust 的 `#![no_std]` async RTOS 内核，采用优先级抢占调度 + 同优先级协程协作的混合调度模型。

## 设计思想

### Executor = 线程

系统中每个 **executor** 绑定一个固定的优先级，在概念上就是一个独立的调度线程。executor 之间通过抢占切换，executor 内部的多个任务通过 `.await` 协作让权。

### 共享系统栈

**所有 executor 共用同一个系统栈**，不为每个 executor 单独分配栈空间。当高优先级 executor 抢占低优先级时，直接在当前栈上继续压栈运行——就像嵌套的函数调用一样：

```
┌──────────────────────────────┐  ← SP 初始位置
│                              │
├──────────────────────────────┤
│  Executor C (prio 0, 最高)   │  ← 抢占 B，SP 继续上移
├──────────────────────────────┤
│  Executor B (prio 2)         │  ← 抢占 A，SP 继续上移
├──────────────────────────────┤
│  Executor A (prio 5, 最低)   │  ← 当前运行的 executor
├──────────────────────────────┤
│  空闲                        │
└──────────────────────────────┘
```

高优先级 executor 完成后栈帧自然归还，`SP` 回退到被抢占的 executor 的位置继续执行。每个 executor 本身就是一个常规 Rust 函数调用，其 `run()` 返回即栈帧销毁。

<details>
<summary>优先级栈（`prio_stack`）</summary>

`Spawner` 内部维护一个 LIFO 优先级栈，记录哪些 executor 当前占据着系统栈空间。栈顶是正在运行的 executor 的优先级。

- `try_preempt()` → 读栈顶作为当前优先级 → 如有更高就绪，push 新高优先级到栈顶
- `run()` 返回后 → `complete_executor()` → pop 栈顶
</details>

### Pend ISR 调度循环

抢占由 Pend（软中断）触发，在中断处理函数中完成 executor 切换：

```
pend_isr:                          // 硬件进入 MIE=0
    save callee-saved regs         // 保存当前 executor 上下文
    loop:                          // MIE=0，临界区
        token = try_preempt()      // 内部使用 critical_section，在 MIE=0 下为冗余但无害
        if is None: break
        enable_interrupts()        // MIE=1，允许被再次抢占
        run(token)                 // 运行新 executor（可被中断）
        disable_interrupts()       // MIE=0，恢复原子操作
        complete_executor()        // 弹出已完成的 executor
    restore callee-saved regs      // 恢复原始 executor 上下文
    mret                           // 返回被抢占的执行流
```

关键设计：`run(token)` 期间 `MIE=1`，允许更高优先级的 executor 在当前 executor 的栈帧之上继续嵌套抢占。Pend ISR 本身运行在共享栈上的被抢占 executor 的栈帧之上，这也是 executor 切换的"零额外栈开销"来源。

> **注**：`try_preempt()` 和 `complete_executor()` 内部使用 `critical_section::with`，在 RISC-V 上该原语通过禁/使中断实现。Pend ISR 循环中 MIE 已经为 0，因此 `critical_section` 的开销仅为增加/减少一个嵌套计数——功能上冗余但无副作用，且保证代码在不依赖中断状态的上下文中也可安全调用。

## 架构

### 调度

- **跨优先级（抢占）：** 高优先级 executor 抢占低优先级，直接在共享栈上嵌套运行。通过 O(1) 两级位图（`PriorityBitmap<G>`）在常数时间内定位最高优先级的就绪 executor。
- **同优先级（协作）：** 同一 executor 上的任务通过 `.await` 让权，FIFO 顺序执行。同一优先级的任务不抢占彼此，共享 executor 的调用栈。

## 特性

- **`#![no_std]`** — 纯 Rust，无 libc 依赖，适合裸机部署
- **任务数量无上限** — 栈由 executor 的调用链自然分配，不逐 task 分配栈
- **零额外栈切换开销** — executor 切换复用 Rust 的函数调用/返回语义，不经汇编上下文切换
- **最多 4,096 个优先级** — `PriorityBitmap<G>` 支持 G ∈ [1, 64]，每组 64 个优先级
- **O(1) 就绪选择** — 两级位图 `trailing_zeros()` 常数时间找到最高优先级的 executor
