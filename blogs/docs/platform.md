---
title: 平台抽象层
date: 2026-07-09
type: description
---

# 平台抽象层

rt-async 通过 `Board` trait + driver model 抽象平台。板级 crate（如 `qemu-virt`、`std-chip`）实现 `Board`，提供设备树与 driver 列表；`platform::driver::boot()` 遍历设备树、按 `compatible` 匹配驱动并实例化；实例化的设备落入一组全局槽位（`CONSOLE`/`TIMER`/...）与多实例注册表（`SERIALS`/`I2C_BUSES`/`SPI_BUSES`），上层经便捷访问器（`console()`/`timer()`）取用。

## Board Trait

`Board` 是板级初始化的入口，经 `#[extern_trait]` 做静态分发（无运行时开销），由 `platform::init()` 调用。

```rust
#[extern_trait(pub BoardImpl)]
pub trait Board {
    fn init();
    /// 延迟初始化：在 app main() 之后、start() 开中断之前调用。
    /// 典型场景：AMP 共享中断控制器需等待另一 hart 完成初始化。
    fn late_init() {}
}
```

| 钩子 | 调用时机 | 职责 |
|------|----------|------|
| `init()` | `platform::init()` 内部 | 注入 DTB、注册 driver 列表、调用 `boot()` 实例化设备 |
| `late_init()` | `platform::start()` 开中断前 | 推迟到最后时刻的板级配置，默认空实现 |

`platform::init()` 流程：初始化 logger → arch 初始化 → `BoardImpl::init()`。

## Driver Model

driver model 把「可被设备树探测的设备」与「设备能提供的功能」正交分离：

- **`Driver` trait** —— 统一的 DT 探测入口。`compatible()` 声明匹配的 DT compatible 列表，`probe(node)` 从节点读 reg/irq 等信息、实例化设备并注册进 registry。driver 实例是零大小单例，`&self` 仅为对象安全。
- **功能 trait** —— 表达「这个设备能做什么」，扁平、可选、组合：`Serial` / `Timer` / `Ipi` / `Reset` / `InterruptController` / `I2cBus` / `SpiBus`。一个设备实现 `Driver`（DT 入口）+ 恰好一个功能能力（CLINT 类设备可同时 impl `Timer` + `Ipi`，作为两个独立单例注册）。

```rust
pub trait Driver: Send + Sync {
    /// 匹配的 compatible 字符串列表（DT 节点任一命中即匹配）。
    fn compatible(&self) -> &'static [&'static str];
    /// 从设备树节点实例化设备并注册进 registry。失败 panic。
    fn probe(&self, node: &Node<'_>);
}
```

`probe()` 内部典型做：从 `node.reg()` 读 MMIO 基址落到全局 `AtomicUsize` → 实例化 → `CONSOLE.set(&INSTANCE)` 或 `SERIALS.register(&INSTANCE)`。功能 trait 方法全部 `&self`（实例方法），支持同类型多设备实例。

## Registry: Slot 与 DeviceRegistry

registry 提供两个原语，承载三类关注点（注册 / 枚举 / 角色）。

### Slot\<T\> —— 单例角色槽位

`Slot<T>` 是 `AtomicU8` 状态机 + `UnsafeCell<MaybeUninit<T>>`。`T` 通常是胖指针（`&'static dyn Serial`），单个 `usize` 放不下，故用 `MaybeUninit` 承载完整胖指针，`Release`/`Acquire` 序保证初始化对读者可见。

```rust
pub struct Slot<T> {
    state: AtomicU8,
    val: UnsafeCell<MaybeUninit<T>>,
}
impl<T> Slot<T> {
    pub const fn new() -> Self { /* ... */ }
    pub fn set(&self, dev: T);       // init 期写入
    pub fn get(&self) -> Option<&T>; // READY 后只读
}
```

6 个单例槽位：`CONSOLE` / `TIMER` / `IPI` / `RESET` / `INTC` / `DRIVERS`。

### DeviceRegistry\<T, N\> —— 多实例枚举

构建在 `[Slot<T>; N]` 数组之上，配线性探测游标。`register` 找首个空槽填入并返回索引，满（探测 N 个均非空）则 panic。

```rust
pub struct DeviceRegistry<T, const N: usize> {
    slots: [Slot<T>; N],
    next: AtomicUsize,
}
impl<T, const N: usize> DeviceRegistry<T, N> {
    pub const fn new() -> Self { /* ... */ }
    pub fn register(&self, dev: T) -> usize;  // 返回索引，满 panic
    pub fn get(&self, idx: usize) -> Option<&T>;
    pub fn iter(&self) -> impl Iterator<Item = &T>;
}
```

多实例注册表（容量 4）：`SERIALS`（`DeviceRegistry<&dyn Serial, 4>`）、`I2C_BUSES`、`SPI_BUSES`。

### 直写式注入与便捷访问器

driver 的 probe **直接**调用 `CONSOLE.set(...)` / `SERIALS.register(...)`，没有 `set_console()` 之类的包装器（这层中间包装已被删除——见技术报告）。上层经便捷访问器取用，未注册则 panic：

```rust
pub fn console() -> &'static dyn Serial;   // *CONSOLE.get().expect(...)
pub fn timer()   -> &'static dyn Timer;
pub fn ipi()     -> &'static dyn Ipi;
pub fn reset()   -> &'static dyn Reset;
pub fn intctl()  -> &'static dyn InterruptController;
```

> **为什么不用 `once_cell`？** `once_cell::sync::OnceCell<T>` 的 `get`/`set` 走 `Option<T>`，对 `&'static dyn Trait` 这类胖指针需要 `AtomicPtr` 承载，而胖指针（数据 + vtable，16 字节）放不进单个 `usize`，故自建 `Slot<T>` 用 `MaybeUninit` 承载完整胖指针。

## 设备树 handoff 与 boot()

`init_dtb` 注入 DTB 切片（来源由板级决定：子模块内嵌 / 主仓库 esos 同款扫描 / handoff），之后 `dt()` 返回全局 `Fdt<'static>` 句柄供 driver probe 使用。`core::cell::OnceCell` 不是 `Sync`，无法直接用作裸机 `static`，故用 `MaybeUninit` + `AtomicBool` 标记（edition 2024 下用 `addr_of_mut!` 取裸指针，避免禁止的 `static mut` 引用）。

`boot()` 是 driver model 的中枢：遍历设备树，对每个节点按 `compatible` 匹配 `DRIVERS` 槽注入的 driver 列表，命中则调 `Driver::probe`。遍历是**深度感知的 DFS 先序**（父先于子），维护一个 bus 栈（见下文 bus 抽象）。

```rust
pub fn boot() {
    let drivers = DRIVERS.get().expect("DRIVERS not set");
    let fdt = crate::dtb::dt();
    bus_stack_reset();
    let mut prev_level = 1;
    for node in fdt.all_nodes() {            // DFS 先序，node.level = 深度
        if node.level < prev_level {         // 离开 controller 子树
            bus_stack_pop_to(node.level);    // 弹出更深的 bus 索引
        }
        for drv in *drivers {
            let matched = node.compatibles()
                .any(|nc| drv.compatible().iter().any(|dc| *dc == nc));
            if matched { drv.probe(&node); }
        }
        prev_level = node.level;
    }
    derive_console(fdt);                     // 从 chosen 选 console
}
```

`derive_console` 在所有节点 probe 完成后调用：各 serial driver 已把实例 `register` 进 `SERIALS`，`derive_console` 从 `chosen { stdout-path }` 派生默认 console。当前为单串口板简化版（`SERIALS` 仅一项直接提升为 console，`chosen.stdout` 仅作「期望有 console」校验）；多串口按 `stdout.node.name` 在 `SERIALS` 中匹配留待未来。`std-chip`（host 桩）不经此路径——无 DTB、不调 `boot()`，直接 `CONSOLE.set(...)`。

## bus 抽象

`I2cBus` / `SpiBus` 是 controller driver 的功能 trait；controller 的 probe 把实例 `register` 进 `I2C_BUSES`/`SPI_BUSES`，返回的索引压入活跃 bus 栈；child device（eeprom/传感器）的 probe 经 `current_i2c()`/`current_spi()` 取栈顶（即最近进入的 controller）收发，不直接 MMIO。

| 函数 | 作用 |
|------|------|
| `bus_stack_reset()` | `boot()` 开始时清空 bus 栈 |
| `push_i2c(idx)` / `push_spi(idx)` | controller probe 注册 bus 后压入索引 |
| `current_i2c()` / `current_spi()` | child probe 取栈顶 bus 实例 |
| `bus_stack_pop_to(level)` | 深度回退时弹出更深的 bus（当前简化：level ≤1 全清） |

bus 栈存的是 `DeviceRegistry::register` 返回的索引（`usize`），不是 `Node`。栈用 `UnsafeCell<BusStack>`（内含两个 `heapless::Vec<usize, 8>`）承载，与 `Slot` 同安全模型，单 hart 串行 boot 下无并发。当前实现对单层 controller 场景正确（controller 挂在总线根下、child 同层或更深）；多层嵌套 controller 需给每个 bus 索引配 level，留作未来。

## 中断分发

`IRQ_TABLE` 是 `[AtomicUsize; 64]`（QEMU virt PLIC 有 53 个源，64 留余量），直接用 IRQ 号做 O(1) 静态数组索引——无哈希、无链表、无排序。handler 类型是 `unsafe fn(u32)` 裸函数指针，无 trait object / vtable。

| 函数 | 作用 |
|------|------|
| `register_irq(irq, handler)` | `board_init` 中为外设 IRQ 注册 handler（关中断完成） |
| `dispatch_external()` | 通用 MachineExternal ISR 入口：`claim()` → 查表 → 调 handler → `complete()` |

`dispatch_external` 经 `driver::intctl()` 取中断控制器做 claim/complete。虚假中断（claim 返回 0）仅 complete 不调 handler。`#[no_mangle] fn __rt_machine_external` 经链接脚本 `PROVIDE(MachineExternal = __rt_machine_external)` 设为默认 MachineExternal handler（弱符号，App 可强符号覆盖）。

仅管理 **MachineExternal** 中断。MachineSoft（调度器）由 executor-macro 强符号接管；MachineTimer 由 `#[executor::interrupt]` 提供。

## 平台模块结构

```
modules/platform/
├── src/
│   ├── lib.rs             # Board trait、init()、start()、pend()/clear_pend()
│   ├── device.rs          # Driver + 功能 trait（Serial/Timer/Ipi/...）
│   ├── driver.rs          # Slot / DeviceRegistry / boot() / derive_console
│   ├── bus.rs             # I2C_BUSES / SPI_BUSES + 活跃 bus 栈
│   ├── dtb.rs             # init_dtb() / dt() 设备树 handoff
│   ├── irq.rs             # IRQ_TABLE[64] / register_irq / dispatch_external
│   ├── logger.rs          # 基于 console() 的 log 实现
│   └── drivers/           # 内置驱动（ns16550a/clint/sifive-plic/...）
├── chips/
│   ├── qemu-virt/         # RISC-V QEMU virt 板级实现
│   └── std-chip/          # host std 单测桩
└── archs/
    └── riscv64-rt/        # arch（中断使能/禁用、TrapFrame、idle）
```

## 现有实现

### StdChip（host 单测桩）

为 `cargo test` 提供 `Board` 实现。无 DTB、不调 `boot()`，直接把桩 driver `set` 进槽位：console 走 `print!`、timer 返回固定值、reset 调 `exit(0)`、ipi 为空操作。

### QemuVirt（RISC-V QEMU virt）

完整的 driver model 路径：内嵌 DTB（`include_bytes!`）→ `init_dtb` → `DRIVERS.set(default_drivers())` → `boot()` 遍历 DT 实例化（ns16550a/clint-timer/clint-msip/sifive-test/sifive-plic）。子仓库 demo/test 为 TX 单测，不依赖外部中断，故不注册 UART RX IRQ handler（中断驱动 RX 由主仓库 chip crate 配置）。

## 移植指南

为新板卡添加支持：

1. **实现 `Board`**：新建 `chips/my-board/` crate，`#[extern_trait] impl Board for MyBoard`。在 `init()` 里：注入 DTB（`init_dtb`）→ `DRIVERS.set(...)` → 调 `boot()`。需要延迟配置的写 `late_init()`。
2. **提供设备树**：内嵌（`include_bytes!`）或运行时扫描/handoff，调 `init_dtb(&'static [u8])`。
3. **组装 driver 列表**：用内置 `default_drivers()` 或自行组装 `&'static [&'static dyn Driver]` 数组替换个别驱动，经 `DRIVERS.set` 注入。
4. **实现驱动**：每个驱动实现 `Driver`（`compatible` + `probe`）+ 恰好一个功能 trait（`Serial`/`Timer`/...）。`probe` 内从 `node.reg()` 取 MMIO、实例化、`set`/`register` 进对应槽位/注册表。
5. **注册中断**：在 `init`/`late_init` 中为各外设 IRQ 调 `register_irq(irq, handler)`（在 `start()` 开全局中断之前）。
6. **在 App 的 feature/链接** 中指向新 Board（`#[extern_trait]` 的实现 crate）。
