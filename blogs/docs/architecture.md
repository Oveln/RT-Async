---
title: 架构概览
date: 2026-05-09
type: description
---

# 架构概览

RT-Async 是一个基于 Rust 的 `#![no_std]` async RTOS 内核，采用优先级抢占 + 同优先级协程协作的混合调度模型。

## 核心设计

### Executor = 线程

系统中每个 **executor** 绑定一个固定优先级，概念上就是一个独立的调度线程。executor 之间通过抢占切换，executor 内部多个任务通过 `.await` 协作让权。

### 共享系统栈

所有 executor 共用同一个系统栈。高优先级 executor 抢占低优先级时，直接在当前栈上继续压栈运行：

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

高优先级 executor 完成后栈帧自然归还，`SP` 回退到被抢占 executor 的位置继续执行。每个 executor 本身就是一个常规 Rust 函数调用，其 `run()` 返回即栈帧销毁。

## 模块结构

```
rt-async/
├── modules/
│   ├── executor/          # 核心调度器
│   │   ├── spawner.rs     #   Spawner<N>：优先级抢占调度器
│   │   ├── executor.rs    #   Executor：单优先级任务执行器
│   │   ├── priority_bitmap.rs  # O(1) 两级优先级位图
│   │   └── task/          #   TaskStorage、RunQueue、TaskInfo
│   ├── executor-macro/    # 过程宏（#[task]、#[main]、#[interrupt]）
│   ├── platform/          # 平台实现（RISC-V QEMU virt、std）
│   └── platform-traits/   # Chip trait 定义
└── apps/
    ├── demo/              # 示例程序
    └── test/              # 集成测试（QEMU 运行）
```

## 调度模型

| 维度 | 策略 | 机制 |
|------|------|------|
| 跨优先级 | 抢占 | 高优先级 executor 抢占低优先级，通过 Pend ISR 在共享栈上嵌套运行 |
| 同优先级 | 协作 | 同 executor 内任务通过 `.await` 让权，FIFO 顺序执行 |

## 特性

- **`#![no_std]`** — 纯 Rust，无 libc 依赖，适合裸机部署
- **任务数量无上限** — 栈由 executor 调用链自然分配，不逐 task 分配栈
- **零额外栈切换开销** — executor 切换复用 Rust 函数调用/返回语义
- **最多 4,096 个优先级** — `PriorityBitmap<G>` 支持 G ∈ [1, 64]
- **O(1) 就绪选择** — 两级位图 `trailing_zeros()` 常数时间定位最高优先级
