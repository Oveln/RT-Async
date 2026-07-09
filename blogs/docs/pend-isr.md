---
title: Pend ISR 调度循环
date: 2026-05-09
type: description
---

# Pend ISR 调度循环

抢占由 Pend（软件中断）触发，在中断处理函数中完成 executor 切换。

## 调度流程

在 RISC-V 上，调度 ISR 为 `MachineSoft`（Machine Software Interrupt）。由 `#[main]` 宏自动生成：

```
MachineSoft ISR:
    clear_pend()                      // 清除 MSI 挂起标志，检查 PEND_MARKER
    if PEND_MARKER == true:           // 调度器触发的 MSI
        loop:                         // MIE=0，临界区
            token = try_preempt()     // 检查是否有更高优先级就绪
            if None: break
            enable_interrupts()       // MIE=1，允许被再次抢占
            run(token)                // 运行新 executor（可被中断）
            disable_interrupts()      // MIE=0，恢复原子操作
            complete_executor()       // 弹出已完成的 executor
    else:                             // 外部 MSI（硬件/跨核）
        __Inner_MachineSoft(tf)       // 调用用户定义的中断处理
```

## PEND_MARKER

`platform::PEND_MARKER` 是一个 `AtomicBool`，用于区分 MSI 来源：

| PEND_MARKER | 来源 | 动作 |
|-------------|------|------|
| `true` | `platform::pend()`（调度器） | 运行优先级抢占调度循环 |
| `false` | 外部 MSI（硬件/跨核 IPI） | 调用 `__Inner_MachineSoft`（用户定义） |

`pend()` 在触发 MSI 前将 `PEND_MARKER` 置 `true`，`clear_pend()` 读取并清除它。

## pend 与 IPI 设备

`pend()` / `clear_pend()` 不再直接操作固定硬件寄存器，而是经 driver registry 取 IPI 设备（`driver::ipi()`）。`pend()` 内部置 `PEND_MARKER` 后调用 `driver::ipi().send()`，`clear_pend()` 在 ISR 早期 `swap` 掉 `PEND_MARKER` 后调用 `driver::ipi().clear()`。具体 IPI 硬件实现（QEMU virt 的 CLINT MSIP / K3 的 PXA IPI）由板级 driver 的 `Ipi` trait 实现决定，平台层与调度器本身与硬件解耦。

## 嵌套抢占

`run(token)` 执行期间 `MIE=1`，允许更高优先级 executor 在当前 executor 的栈帧之上继续嵌套抢占。这是共享系统栈设计的关键：

1. Executor A (prio 5) 正在运行
2. 更高优先级任务就绪 → `pend()` 触发 MSI
3. `MachineSoft` ISR 挂起当前执行流，保存 callee-saved 寄存器
4. `try_preempt()` 返回 RunToken(prio 2)
5. `enable_interrupts()` → `run(token)` 执行 Executor B (prio 2)
6. Executor B 运行期间又触发 MSI → 重复步骤 3-5 执行 Executor C (prio 0)
7. Executor C 完成 → `disable_interrupts()` → `complete_executor()` → 回到 Executor B
8. Executor B 完成 → 回到 Executor A → `restore callee-saved regs` → `mret`

## critical_section 与中断状态

`try_preempt()` 和 `complete_executor()` 内部使用 `critical_section::with`。在 RISC-V 上该原语通过禁/使中断实现。Pend ISR 循环中 MIE 已经为 0，因此 `critical_section` 的开销仅为增加/减少一个嵌套计数——功能上冗余但无副作用，保证代码在不依赖中断状态的上下文中也可安全调用。
