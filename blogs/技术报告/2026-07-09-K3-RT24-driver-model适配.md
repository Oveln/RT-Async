---
title: "K3 RT24 rcpu1：从 Chip 硬编码到 Board + Driver Model"
date: 2026-07-09
category: 架构设计
---

# K3 RT24 rcpu1：从 Chip 硬编码到 Board + Driver Model

## 背景

[上一篇文章](./2026-07-08-driver-model与DTB-handoff现代化.md) 在 QEMU virt 线上完成了 platform 层的 driver model 重构，将 `Chip`/`TimerChip` 两个硬编码 MMIO 的 trait 替换为 `Board` + `Driver`（DT probe 实例化）。但 K3 RT24 线仍然沿用旧的 `impl Chip` / `impl TimerChip` 模式——`uart.rs` 里地址常量写死，所有外设初始化全部堆在 `board_init()` 一条路径里。

本次提交将 K3 线**整体迁移到新 driver model**，同时把 app 从原来的 `minimal`（只验证串口 hello）升级为 `sched_demo`（跑通抢占调度 + SysTimer 定时器 + MSIP 自中断全链路）。改动量：主仓库 10 个文件（+96 / -144），新增 5 个源文件 + 2 个设备树文件。

## 文件级变更总览

| 文件 | 变更 |
|---|---|
| `modules/chip-k3-rt24/src/lib.rs` | `impl Chip`/`TimerChip` → `impl Board`，去掉手动 put_str / set_deadline |
| `modules/chip-k3-rt24/src/uart.rs` | **删除**（63 行硬编码驱动，被 pxa_uart.rs 替代） |
| `modules/chip-k3-rt24/src/pxa_uart.rs` | **新增** —— `Serial` trait + `Driver` trait 双实现，经 DT probe 初始化 |
| `modules/chip-k3-rt24/src/clint_k3.rs` | **新增** —— SysTimer 驱动：`Timer`（mtime/mtimecmp）+ `Ipi`（MSIP） |
| `modules/chip-k3-rt24/src/plic_k3.rs` | **新增** —— PLIC 驱动，适配 K3 非标准寄存器布局 |
| `modules/chip-k3-rt24/src/reset_stub.rs` | **新增** —— no-op `Reset` trait（K3 无复位外设，wfi 死循环占位） |
| `modules/chip-k3-rt24/Cargo.toml` | +fdt-parser +log；riscv 限定 `critical-section-single-hart` |
| `apps/rt-async-k3/src/bin/minimal.rs` | **删除** → 替换为 `sched_demo.rs` |
| `apps/rt-async-k3/src/bin/sched_demo.rs` | **新增** —— 双任务抢占调度 demo（task_high / task_low，SysTimer + MSIP） |
| `apps/rt-async-k3/Cargo.toml` | +futures + fugit；bin name `minimal` → `sched_demo` |
| `apps/rt-async-k3/build.rs` | hart stack size 4096 → 8192 |
| `Cargo.toml`（workspace） | 注册 `chip-k3-rt24` / `rt-async-k3` |
| `amp.toml` | +K3 外设基址（SYSTIMER / PLIC / UART0 / IRQ） |
| `xtask/src/build.rs` | bin name `minimal` → `sched_demo`，产物 `rt-async-k3-minimal.elf` → `rt-async-k3-sched-demo.elf` |
| `its/rt-async-k3.dts` | **新增** —— rcpu1 设备树（内嵌进 ELF .rodata） |
| `its/rt-async-k3.dtb` | **新增** —— 编译产物 |

## 核心设计：K3 RT24 的 Board 实现

重构前，`chip-k3-rt24` 的初始化是一条线性调用链：

```rust
// 重构前：lib.rs —— 所有外设硬编码在 Chip/TimerChip 实现里
#[extern_trait]
impl Chip for K3Rt24 {
    fn board_init() {
        clock::early_init();   // SPL 握手 + 时钟 + pinmux
        uart::init();          // 硬编码 UART0_BASE=0xc0881000
    }
    fn put_str(s: &str) {
        for &b in s.as_bytes() { uart::putc(b) }  // 轮询写 THR
    }
}
#[extern_trait]
impl TimerChip for K3Rt24 {
    fn freq_hz() -> u32 { 0 }        // stub：minimal 无定时器
    fn now_ticks() -> u64 { 0 }
    fn set_deadline(_: u64) {}
}
```

重构后，`Board::init()` 只做**编排**：注入 DTB → 注册 driver 列表 → DT 遍历自动 probe 各外设 → 最后补上无 DT 节点的 reset stub。

```rust
// 重构后：lib.rs —— Board trait，外设由 driver model 接管
static K3_DRIVERS: &[&dyn Driver] = &[
    &pxa_uart::INSTANCE,   // Serial
    &clint_k3::TIMER,      // Timer
    &clint_k3::MSIP,       // Ipi
    &plic_k3::PLIC,        // InterruptController
];

#[extern_trait]
impl Board for K3Rt24 {
    fn init() {
        clock::early_init();                                          // ① 握手+时钟+pinmux
        platform::dtb::init_dtb(include_bytes!("...(省略)...dtb"));   // ② 注入内嵌 DTB
        platform::driver::set_drivers(K3_DRIVERS);                   // ③ 注册 driver 列表
        platform::driver::boot();                                    // ④ DT 遍历 → probe → 写 registry
        platform::driver::set_reset(&reset_stub::INSTANCE);          // ⑤ 无 DT 节点，直接注册
    }
}
```

`pxa_uart`、`clint_k3`（Timer/Ipi）、`plic_k3` 三个 driver 各自实现 `Driver` trait 的 `compatible()` + `probe()`，由 `boot()` 遍历 DT 节点后按 compatible 字符串自动匹配实例化。probe 成功后调用 platform 提供的 registry setter（`set_console` / `set_timer` / `set_ipi` / `set_intctl`）登记到全局槽位，上层通过 `platform::console()` / `platform::timer()` 等访问器透明调用，无需感知底层是 QEMU ns16550a 还是 K3 PXA-UART。

## K3 专属驱动要点

### 1. PXA-UART（`pxa_uart.rs`）

实现 `Serial` trait（`write`/`read`/`has_data`），compatible = `"spacemit,pxa-uart0"`。与旧 `uart.rs` 的差异：

- **不再硬编码 `UART0_BASE`**：基址从 DT `reg` 属性读取，存储在全局 `AtomicUsize` 中，所有 MMIO 操作以 BASE + offset 方式寻址。
- **新增 `Serial::read()` / `has_data()`**：为后续异步 RX（`SerialRx`）预留轮询读路径。
- **probe 顺序约束**：UART probe 必须先于其他 driver 完成（日志通过 console 输出）。因此 `K3_DRIVERS` 数组把 `pxa_uart::INSTANCE` 放在首位，boot() 按序 probe。

### 2. SysTimer（`clint_k3.rs`）

K3 RT24 的 SysTimer 采用**非标准 per-hart 步长**：`win = base + (hart << 27)`（标准 SiFive CLINT 是 `mtimecmp hart*8`）。关键寄存器地址（rcpu1 = hart 1）：

| 寄存器 | 地址 | 说明 |
|---|---|---|
| mtime | `base + 0xbff8` = `0xe400bff8` | 全局，所有 hart 共享；实测读到递增计时 |
| mtimecmp | `win + 0x4000` = `0xec004000` | per-hart，先写高 32 位再写低 32 位防止伪触发 |
| MSIP | `win + 0x0` = `0xec000000` | 上板实测：写 1 → MachineSoft 触发，mip=0x8 |

`K3SysTimer` 实现 `Timer` trait（`freq_hz()` / `now()` / `set_deadline()`），`K3Msip` 实现 `Ipi` trait（`send()` / `clear()`）。两者都实现 `Driver`，共享同一 `WIN` 全局原子（底座相同、hart 相同），compatible 分别声明为 `"spacemit,k3-systimer"` 和 `"spacemit,k3-systimer-msip"`（回退 `"riscv,clint0"` / `"riscv,clint0-msip"`）。

hart id 从 FDT `boot_cpuid_phys` 获取（dtc 从 `/cpus/cpu@1` 的 `reg=<1>` 推导），与 QEMU 侧 driver 同一机制，同一份代码无需改动即可适配 rcpu0/rcpu1。

### 3. PLIC（`plic_k3.rs`）

K3 RT24 的 PLIC 与 SiFive 标准 PLIC 的差异如下：

| 特性 | 标准 SiFive PLIC | K3 RT24 PLIC |
|---|---|---|
| per-hart 步长 | `context = hart * 2`（小步长） | **`hart << 27`**（大步长） |
| priority 寄存器 | **全局共享**（非 per-hart） | **per-hart**（带 win 偏移） |
| base+0x0 | 保留/只读 | **feature 寄存器**（可写） |
| claim/complete | 通常分开 | **共用同一寄存器**（写即 complete） |

compatible 声明为 `"riscv,plic0"`（与标准 PLIC 相同），但 probe 内部按 K3 布局计算偏移。支持 `enable_irq` / `disable_irq` / `set_priority` / `set_threshold` / `claim` / `complete` 全套操作，为后续 UART RX 中断驱动做基础设施准备。

### 4. K3 设备树（`its/rt-async-k3.dts`）

与 QEMU 线不同，**U-Boot 对 RT24 不做 DTB handoff**（`k3-rproc.c` 只 memcpy ELF 的 PT_LOAD 段），因此 DTB 必须内嵌进 ELF：

```
Board::init()
  → platform::dtb::init_dtb(include_bytes!("...(省略)...dtb"))
    // include_bytes! 把 .dtb 编进 .rodata，作为 PT_LOAD 段随 ELF 加载
```

DT 描述 rcpu1（hart 1）视角的外设：

| 节点 | compatible | reg |
|---|---|---|
| `/cpus/cpu@1` | `spacemit,rt24`, `riscv` | hart=1 |
| `/serial@c0881000` | `spacemit,pxa-uart0` | `0xc0881000` |
| `/systimer@e4000000` | `spacemit,k3-systimer`, `riscv,clint0` | `0xe4000000` |
| `/msip@e4000000` | `spacemit,k3-systimer-msip`, `riscv,clint0-msip` | `0xe4000000` |
| `/intc@e0000000` | `riscv,plic0` | `0xe0000000` |

MSIP 与 systimer 共享同一 reg 区间（同为 `0xe4000000`），但分属不同 DT 节点+不同 compatible，boot() 会分别调用各自的 `probe()` 注册到独立槽位。

## App 升级：minimal → sched_demo

```rust
// sched_demo.rs —— 在 K3 真板上验证抢占调度全链路

#[executor::task]
async fn task_high() {
    loop {
        futures::timer::after(50.millis()).await;  // SysTimer → TimerQueue → 唤醒
        platform::console().write(b"H");            // pxa_uart::INSTANCE.write()
        // 每 20 次打一行计数
    }
}

#[executor::task]
async fn task_low() {
    loop {
        futures::timer::after(50.millis()).await;  // 同频率，低优先级
        platform::console().write(b"L");
    }
}

#[executor::main]
fn main(spawner: Pin<&'static Spawner<4>>) {
    spawner.spawn(Priority::new(3), task_high().unwrap());  // 高优先级
    spawner.spawn(Priority::new(1), task_low().unwrap());   // 低优先级
}

// ISR
#[executor::interrupt] fn MachineSoft(_tf: &mut TrapFrame) {}    // Ipi::send() → 调度
#[executor::interrupt] fn MachineTimer(_tf: &mut TrapFrame) {    // Timer::fired → after()
    futures::timer::handle_timer_isr();
}
```

运行效果：串口交替输出 `H` 和 `L`，H 出现频率不低于 L（task_high 抢占 task_low），证明定时器唤醒 + 优先级抢占 + MSIP 自中断全链路在 K3 真板上跑通。

## 小结

本次提交完成了两条线在 driver model 上的统一：

| | QEMU virt 线 | K3 RT24 线 |
|---|---|---|
| 驱动接入 | `default_drivers()`（ns16550a / sifive-test / CLINT） | 自定义 `K3_DRIVERS`（pxa-uart / SysTimer / PLIC） |
| DTB 来源 | esos 同款扫描（QEMU loader 摆 DTB） | **内嵌** `include_bytes!`（U-Boot 不 handoff） |
| Board trait | `impl Board for QemuVirtRt` | `impl Board for K3Rt24` |

两条线的 `Board` trait 签名完全一致——差异仅在 `set_drivers` 传入的 driver 列表和 DTB 来源，driver model 核心逻辑零重复代码。为后续 Zephyr / embassy 风格的外设驱动开发（设备树描述硬件 + Driver trait 统一 probe）奠定了基础设施。
