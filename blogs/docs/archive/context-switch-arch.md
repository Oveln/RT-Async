---
title: 上下文切换架构设计（历史）
date: 2025-11-29
type: description
---

> **注意**：本文档描述的是前身项目 Embassy Preempt 的上下文切换设计。RT-Async 已采用共享系统栈方案，executor 切换不再需要独立的上下文保存/恢复。当前设计请参考 [Pend ISR 调度循环](../pend-isr.md)。

# Embassy Preempt 上下文切换架构设计

## 核心设计原则

**所有平台的寄存器保存都在进入`__ContextSwitchHandler`函数中通过`save_task_context()`统一完成**

## 架构流程图

```mermaid
graph TD
    A[调度器触发上下文切换] --> B{平台架构}

    %% ARM Cortex-M 路径
    B -->|ARM Cortex-M| C[NVIC寄存器设置PendSV]
    C --> D[PendSV异常自动触发]
    D --> E[硬件自动保存部分寄存器<br/>R0-R3, R12, LR, PC, xPSR]
    E --> F[__ContextSwitchHandler统一入口]

    %% RISC-V 路径
    B -->|RISC-V| G[ecall指令触发]
    G --> H[MachineEnvCall异常处理]
    H --> I[仅做栈切换<br/>csrrw sp, mscratch, sp]
    I --> F

    %% 统一处理
    F --> J[调用Platform::save_task_context]
    J --> K[补全/保存所有寄存器到栈]
    K --> L[执行平台无关的调度逻辑]
    L --> M[选择最高优先级任务]
    M --> N[调用Platform::restore_task_context]
    N --> O[恢复寄存器并切换到新任务]
```

## 详细时序图

```mermaid
sequenceDiagram
    participant App as 应用程序
    participant Sched as 调度器
    participant IRQ as 异常处理
    participant CS as __ContextSwitchHandler
    participant PF as Platform函数

    App->>Sched: 触发上下文切换

    alt ARM平台
        Sched->>IRQ: 设置NVIC寄存器
        Note over IRQ: PendSV硬件触发
        IRQ->>IRQ: 自动保存部分寄存器
        IRQ->>CS: 跳转到__ContextSwitchHandler
    else RISC-V平台
        Sched->>IRQ: 执行ecall指令
        IRQ->>IRQ: csrrw sp, mscratch, sp (仅栈切换)
        IRQ->>CS: 跳转到__ContextSwitchHandler
    end

    CS->>PF: 调用save_task_context()
    Note over PF: 统一保存所有寄存器
    PF->>PF: 完成上下文保存
    PF->>CS: 返回

    CS->>CS: 执行平台无关调度逻辑
    CS->>CS: 选择新任务
    CS->>PF: 调用restore_task_context()
    Note over PF: 统一恢复寄存器
    PF->>PF: 完成上下文恢复
    PF->>App: 返回到新任务
```

## 关键设计正确性验证

### 1. 统一的入口点
```rust
// 两个平台最终都进入相同的函数
#[unsafe(no_mangle)]
extern "C" fn __ContextSwitchHandler() {
    // 1. 统一调用平台相关的上下文保存
    unsafe {
        embassy_preempt_platform::PlatformImpl::save_task_context();
    }

    // 2. 执行平台无关的调度逻辑
    let global_executor = GlobalSyncExecutor().as_ref().unwrap();
    // ... 调度算法
}
```

### 2. RISC-V异常入口的最小化设计
```assembly
# 正确：只做栈切换，不保存寄存器
MachineEnvCall:
    csrrw sp, mscratch, sp    # 仅切换栈指针
    j __ContextSwitchHandler # 跳转到统一处理函数
```

### 3. ARM和RISC-V的一致性保证
- **ARM**: PendSV + 部分硬件保存 + `save_task_context()`补全
- **RISC-V**: ecall + 栈切换 + `save_task_context()`完整保存
- **结果**: 两个平台都以相同状态进入调度器
