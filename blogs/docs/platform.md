---
title: 平台抽象层
date: 2026-05-09
type: description
---

# 平台抽象层

RT-Async 通过 `platform-traits` 定义 `Chip` trait，将平台相关操作抽象为统一接口。

## Chip Trait

```rust
pub trait Chip {
    fn shutdown() -> !;
    fn put_str(s: &str);
    unsafe fn pend();
    unsafe fn clear_pend();
}
```

| 方法 | 说明 |
|------|------|
| `shutdown()` | 关机（成功退出） |
| `put_str(s)` | 串口输出 |
| `pend()` | 触发调度器软件中断 |
| `clear_pend()` | 清除软件中断挂起标志 |

## 平台模块结构

```
platform/
├── lib.rs               # 公共接口：init()、pend()、clear_pend()、PEND_MARKER
├── logger.rs            # 基于 Chip::put_str 的 log 实现
└── chips/
    ├── std-chip/        # 标准库环境（测试用）
    └── qemu-virt/       # RISC-V QEMU virt 平台
```

## platform 模块公共接口

```rust
// 初始化（logger + 平台）
pub fn init();

// 使能 MSI + 开全局中断（仅 qemu-virt feature）
pub unsafe fn start();

// 调度器 pend 标记
pub static PEND_MARKER: AtomicBool;

// 触发调度器软件中断（设置 PEND_MARKER 后调用 Chip::pend）
pub unsafe fn pend();

// 清除 pend 标志，返回 PEND_MARKER 先前值（区分调度/外部 MSI）
pub unsafe fn clear_pend() -> bool;
```

## 现有实现

### StdChip（`std` feature）

用于在标准库环境下测试。`pend()` / `clear_pend()` 为空操作，`shutdown()` 调用 `std::process::exit(0)`。

### QemuVirt（`qemu-virt` feature）

针对 QEMU `virt` 机器的 RISC-V 实现，直接操作 CLINT 寄存器触发/清除 MSI。

## 移植指南

为新的 SoC/板卡添加支持：

1. 创建新的 chip crate（如 `chips/my-board/`）
2. 实现 `Chip` trait
3. 在 `platform/lib.rs` 中添加 feature gate 指向新实现
4. 提供 `arch` 模块（中断使能/禁用、TrapFrame 定义、idle 等）
