---
title: 共享系统栈
date: 2026-05-09
type: description
---

# 共享系统栈

RT-Async 最核心的设计：所有 executor 共用同一个系统栈，不逐 executor 分配独立栈空间。

## 原理

传统 RTOS 为每个线程分配独立栈，栈切换通过保存/恢复全部 callee-saved 寄存器完成（汇编上下文切换）。RT-Async 的 executor 切换复用 Rust 的函数调用/返回语义：

- **抢占** = 在被抢占 executor 的栈帧之上调用新 executor 的 `run()`（像嵌套函数调用）
- **返回** = executor `run()` 返回，栈帧自然销毁，SP 回退

```
时间线 →

栈空间     ┌─────────────┐
  ↑        │  Executor C  │  ← 抢占 B 后运行
  │        ├─────────────┤
  │        │  Executor B  │  ← 抢占 A 后运行
  │        ├─────────────┤
  │        │  Executor A  │  ← 最初运行
  │        ├─────────────┤
  │        │  空闲        │
            └─────────────┘
```

## prio_stack

`Spawner` 内部维护一个 LIFO 优先级栈（`heapless::Vec<Priority, N>`），记录哪些 executor 当前占据系统栈空间：

- `try_preempt()` → 读栈顶作为当前优先级 → 若有更高就绪，push 新优先级到栈顶
- `run()` 返回后 → `complete_executor()` → pop 栈顶

栈顶始终是当前运行的 executor 的优先级。

## 零额外栈切换开销

executor 切换不需要保存/恢复 callee-saved 寄存器组。切换过程完全由 Pend ISR 中的 `save callee-saved regs` / `restore callee-saved regs` 完成——这是唯一需要汇编的地方，且只在 ISR 入口/出口各执行一次，不随 executor 嵌套深度增加。

嵌套的 executor 直接在栈上增长，自然受到系统栈大小的约束。系统栈需要预留足够空间以容纳最深的嵌套场景。

## 任务数量无上限

传统 RTOS 的任务数量受限于预先分配的栈数量。RT-Async 中任务不拥有独立栈——它们共享所属 executor 的调用栈。栈空间由 executor 的调用链（`run()` → poll Future → `.await`）动态分配，与任务数量无关。
