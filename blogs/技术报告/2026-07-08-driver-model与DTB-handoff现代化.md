---
title: "rt-async 驱动架构现代化：设备树 handoff + Driver Model"
date: 2026-07-08
category: 架构设计
---

# rt-async 驱动架构现代化：设备树 handoff + Driver Model

## 背景：为什么要做这件事

rt-async 原先的 platform 层是 `chip + arch` 架构——每个板级 crate（`chip-qemu-virt-rt`）在 `impl Chip` / `impl TimerChip` 里**硬编码**所有外设的 MMIO 逻辑：

```rust
// 重构前：UART1 地址、CLINT 偏移全部硬编码在 chip crate
fn put_str(s: &str) {
    for &byte in s.as_bytes() {
        unsafe {
            while read_volatile((UART1BASE + LSR) & THRE == 0) {}
            write_volatile(UART1BASE, byte);     // 0x10002000 写死
        }
    }
}
fn set_deadline(tick: u64) {
    unsafe { write_volatile((CLINTBASE + 0x4008) as *mut u64, tick) };  // hart1 偏移写死
}
```

这套做法的问题：

1. **换板子 = 改 chip crate**。UART 地址、CLINT 偏移、时钟频率全散在各方法里，迁移到 K3 等真板需要逐行改。
2. **外设信息与代码耦合**。设备地址是编译期常量，无法从设备树动态获取。
3. **加驱动没有统一模式**。新外设的初始化逻辑得手动塞进 `board_init`，没有约定俗成的接入点。

现代化的 RTOS（Zephyr、embassy、Linux）普遍采用 **设备树描述硬件 + driver model 按需实例化** 的架构。本次重构把 rt-async 的 platform 层改造成了同样的模式，同时保持上层（executor / futures / 宏 / 全部 19 个 bin）**零改动**。

## 总览：改了什么

```
重构前                                    重构后
──────                                    ──────
chip crate (硬编码 MMIO)                  chip crate (转发 shim)
  ├─ board_init() {}  (空)                  ├─ board_init: init_dtb + set_drivers + boot
  ├─ put_str → 直接写 UART1                 ├─ put_str → driver::console().write()
  ├─ set_deadline → 直接写 mtimecmp         ├─ set_deadline → driver::timer().set_deadline()
  └─ 每个板子各写一遍                        └─ 所有板子共用 driver model
                                          
                                          platform (新增 driver model)
                                            ├─ dtb.rs     DTB 注入点 (init_dtb + dt)
                                            ├─ device.rs  Driver/Serial/Timer/Ipi/Reset trait
                                            ├─ driver.rs  registry (Slot<T> + boot + 访问器)
                                            └─ drivers/   内置驱动 (NS16550A/CLINT/sifive)
```

**改动规模**：子模块 +796 行 / -30 行（14 文件），主仓库 +241 行 / -26 行（9 文件）。

### 三条运行线，统一 handoff 抽象

rt-async 实际有三条运行环境，DTB 来源各不同：

| 运行线 | 环境 | DTB 来源 | handoff 代码 |
|---|---|---|---|
| **子模块 demo/test** | QEMU `-kernel -bios none`（hart0） | **内嵌** `include_bytes!` | build.rs 编译进 ELF |
| **主仓库 rt-async-amp** | QEMU + OpenSBI（hart1, M-mode） | **esos 同款扫描** | QEMU loader 摆 DTB + 运行时扫描认领 |
| **executor 单元测试** | host `std` | **无 DTB**（`std-chip`） | — |

核心抽象：`platform` 提供一个 DTB 注入点 `init_dtb(&'static [u8])`，三条线各自把 DTB 喂进去，**driver model 的逻辑完全一致**。

## 架构分层

```
应用层 (apps/ — 零改动)
  │  platform::init(log_level)   ← 签名不变
  │  #[executor::main] / #[executor::interrupt]  ← 宏 codegen 不变
  │  futures::timer::after().await  ← async API 不变
  ▼
platform::init() → ChipImpl::board_init()
  │
  ├── ① init_dtb(slice)         ← DTB 注入（来源由板级决定）
  ├── ② set_drivers(&[&dyn Driver])  ← 注册板级 driver 列表
  └── ③ boot()                  ← 遍历 DT，按 compatible 匹配 → probe
        │
        ▼
driver registry (全局 Slot<T> 槽位)
  ├─ CONSOLE: &dyn Serial   ← NS16550A probe 后填充
  ├─ TIMER:  &dyn Timer     ← ClintTimer probe 后填充
  ├─ IPI:    &dyn Ipi       ← ClintMsip probe 后填充
  └─ RESET:  &dyn Reset     ← SifiveTest probe 后填充
        │
        ▼ (上层访问器)
ChipImpl/TimerChipImpl (extern_trait shim — 纯转发)
  ├─ put_str(s)     → console().write(s)
  ├─ set_deadline(t) → timer().set_deadline(t)
  ├─ pend()         → ipi().send()
  └─ ...
```

调用顺序天然正确：`init()` → `board_init()` → `init_dtb()` → `set_drivers()` → `boot()`（probe 各 driver）→ registry 就绪 → 上层使用。

## 核心组件

### 1. DTB 解析层（`platform/src/dtb.rs`）

提供全局 FDT 句柄，是 driver model 的数据源。

```rust
pub fn init_dtb(dtb: &'static [u8]) {
    // AtomicU8 状态机 + MaybeUninit<Fdt>，单 hart 串行 init
    STATE.compare_exchange(UNINIT, INITING, AcqRel, Acquire)
        .expect("init_dtb: already initialized");
    let fdt = Fdt::from_bytes(dtb).expect("invalid FDT blob");
    unsafe { FDT_PTR.write(fdt); }
    STATE.store(READY, Release);
}

pub fn dt() -> &'static Fdt<'static> {
    // driver probe 时调，返回全局 FDT 句柄
}
```

**为什么不用 `OnceCell`**：`core::cell::OnceCell` 不是 `Sync`，不能作裸机 `static`。用 `AtomicU8` + `MaybeUninit` 手搓等价物，配 `Release`/`Acquire` 序保证初始化结果对后续读者可见。

### 2. Driver 契约（`platform/src/device.rs`）

定义了"可被 DT 探测"的统一入口 + 四个功能 trait：

```rust
pub trait Driver: Send + Sync {
    fn compatible(&self) -> &'static [&'static str];  // 匹配 DT compatible
    fn probe(&self, node: &Node<'_>);                  // 从 DT 读 reg，实例化并注册
}

pub trait Serial: Send + Sync { fn write(&self, buf: &[u8]); }
pub trait Timer:  Send + Sync { fn freq_hz(&self)->u32; fn now(&self)->u64; fn set_deadline(&self, tick:u64); }
pub trait Ipi:    Send + Sync { unsafe fn send(&self); unsafe fn clear(&self); }
pub trait Reset:  Send + Sync { fn shutdown(&self) -> !; }
```

**设计要点**：
- `Driver` 是"可被 DT 探测"的入口，功能 trait 是"这个设备能做什么"。一个设备可同时实现多个功能（CLINT 同时 impl Timer + Ipi）。
- 全部 `&self`（对象安全），registry 用 `&dyn Trait` trait 对象存储。
- driver 实例是零大小单例，`&self` 无运行时开销，probe 内部通过全局 `AtomicUsize` 落地 MMIO 基址。

### 3. Registry（`platform/src/driver.rs`）

全局槽位持有已实例化的设备，提供便捷访问器。

```rust
// 胖指针槽位：UnsafeCell<MaybeUninit<T>> + AtomicU8 状态机
struct Slot<T> {
    state: AtomicU8,
    val: UnsafeCell<MaybeUninit<T>>,
}
unsafe impl<T: Send> Sync for Slot<T> {}

static CONSOLE: Slot<&'static dyn Serial> = Slot::new();
static TIMER:  Slot<&'static dyn Timer>   = Slot::new();
static IPI:    Slot<&'static dyn Ipi>     = Slot::new();
static RESET:  Slot<&'static dyn Reset>   = Slot::new();
static DRIVERS: Slot<&'static [&'static dyn Driver]> = Slot::new();

pub fn boot() {
    let drivers = DRIVERS.get().expect("set_drivers not called");
    for node in dt().all_nodes() {
        for drv in *drivers {
            if node.compatibles().any(|nc| drv.compatible().iter().any(|dc| *dc == nc)) {
                drv.probe(&node);  // 命中则 probe → 填充对应槽位
            }
        }
    }
}
```

**为什么需要 `Slot<T>`**：`&dyn Trait` 是胖指针（数据指针 + vtable 指针 = 16 字节），单个 `AtomicUsize` 放不下。`MaybeUninit<T>` 承载完整胖指针，`UnsafeCell` 提供内部可变性（让 `&self` 能在 init 期写入），`AtomicU8` 状态机保证可见性。

**`console()` 的降级处理**：未注册时返回 `NoOpSerial`（静默丢弃），而非 panic。防止 panic handler 在 console 缺失时触发二次 panic。timer/ipi/reset 保持 panic（它们无静默降级语义）。

### 4. 内置驱动（`platform/src/drivers/`）

四个驱动，全部零大小单例 + 全局 `AtomicUsize` 存 probe 来的 MMIO 基址：

| 驱动 | 文件 | compatible | 功能 | 备注 |
|---|---|---|---|---|
| NS16550A | `serial_ns16550a.rs` | `ns16550a` | Serial | 逐字节写 THR，QEMU 即写即收 |
| CLINT Timer | `timer_clint.rs` | `riscv,clint0` | Timer | **hart 感知**：mtimecmp = base + 0x4000 + hart×8 |
| CLINT MSIP | `ipi_clint_msip.rs` | `riscv,clint0-msip` | Ipi | **hart 感知**：msip = base + hart×4 |
| SiFive Test | `reset_sifive_test.rs` | `sifive,test1` | Reset | 写 0x5555 触发 QEMU 退出 |

#### hart 感知：同一份驱动服务 hart0 和 hart1

这是整个设计的关键巧思。CLINT 的寄存器按 hart 编号排列，hart0 和 hart1 的偏移不同。重构前是硬编码（子模块写 hart0 偏移，主仓库写 hart1 偏移）；重构后从 **FDT header 的 `boot_cpuid_phys`** 动态推导：

```rust
// timer_clint.rs 的 probe
fn probe(&self, node: &Node<'_>) {
    let reg = node.reg().next().expect("missing reg");
    BASE.store(reg.address as usize, Release);

    let hart = node.fdt().boot_cpuid_phys() as usize;  // ← 关键
    let mtimecmp_off = 0x4000 + hart * 8;
    OFF_MTIMECMP.store(mtimecmp_off, Release);
    // ...
}
```

`boot_cpuid_phys` 由 dtc 从 `/cpus` 节点推导：
- 子模块 DTS 只有 `cpu@0` → `boot_cpuid_phys = 0` → mtimecmp 在 base+0x4000
- 主仓库 DTS 只有 `cpu@1` → `boot_cpuid_phys = 1` → mtimecmp 在 base+0x4008

**一份驱动代码，两个 hart，零硬编码**。

#### 加新驱动的步骤

```rust
// 1. 新建 drivers/my_driver.rs，impl Driver + 功能 trait
impl Driver for MyDriver {
    fn compatible(&self) -> &'static [&'static str] { &["my,vendor"] }
    fn probe(&self, node: &Node<'_>) {
        let reg = node.reg().next().expect("missing reg");
        BASE.store(reg.address as usize, Release);
        driver::set_console(&INSTANCE);  // 或 set_timer/set_ipi/set_reset
    }
}
impl Serial for MyDriver { fn write(&self, buf: &[u8]) { /* ... */ } }

// 2. drivers/mod.rs 里 pub mod + pub use + 加入 DEFAULT 数组
static DEFAULT: &[&dyn Driver] = &[
    &serial_ns16550a::INSTANCE,
    &my_driver::INSTANCE,      // ← 加这一行
    // ...
];
```

无宏、无 link section、无注册函数。加一个驱动 = 写 `impl Driver` + 加进数组。

## DTB Handoff：两条路径

### 子模块（hart0）：内嵌

最简路径，DTB 编译进 ELF：

```rust
// chips/qemu-virt/src/lib.rs 的 board_init
fn board_init() {
    static RT_ASYNC_DTB: &[u8] = include_bytes!("../../../../../its/rt-async-qemu-virt.dtb");
    platform::dtb::init_dtb(RT_ASYNC_DTB);   // ← 内嵌
    platform::driver::set_drivers(platform::drivers::default_drivers());
    platform::driver::boot();
}
```

`cargo run` 一条命令搞定，保持子模块自包含。

### 主仓库（hart1）：esos 同款扫描

rt-async 跑在 hart1，DTB 由 xtask 经 QEMU `-device loader` 摆进内存，board_init 运行时扫描认领：

```rust
// chip-qemu-virt-rt/src/lib.rs
fn locate_rtasync_dtb() -> &'static [u8] {
    let base = amp::RTASYNCDTBBASE;  // 0x83000000
    for i in 0..16 {                  // 扫 16 页 × 4KB
        let addr = base + i * 0x1000;
        let probe = unsafe { slice::from_raw_parts(addr as *const u8, 0x1000) };
        let Ok(fdt) = Fdt::from_bytes(probe) else { continue };
        let total = fdt.total_size();
        let dtb = unsafe { slice::from_raw_parts(addr as *const u8, total) };
        if Fdt::from_bytes(dtb).ok()?.find_compatible(&["ov,rt-async"]).next().is_some() {
            return dtb;  // 认领 compatible="ov,rt-async" 的 DTB
        }
    }
    panic!("no DTB found");
}
```

**为什么扫描而非固定地址**：QEMU `-device loader` 把 DTB 摆到 `addr` 附近（4KB 对齐即可），不保证精确落在 base。按页步长扫兜底。

**专属 DTB**：rt-async 获得的是自己的独立 DTB（只含 uart/timer/ipi/reset 节点，不含 PLIC/virtio/PCI），通过 root 节点 `compatible = "ov,rt-async"` 鉴别——与 StarryOS 使用的整机 DTB 是两个文件。这参考了 U-Boot k3→esos 的 handoff 方案。

## 关键 bug 排查：BSS 未清零

整个重构过程中最隐蔽的问题。现象：主仓库全链路启动时，rt-async 在 `board_init` 里调用 `init_dtb` 时，`compare_exchange(UNINIT → INITING)` **误判为"已初始化"**而 panic。

### 根因

rt-async 经 QEMU `-device loader` 加载，BSS 段在链接脚本里是 `NOLOAD`（不占文件空间，loader 不初始化）。driver model 引入了依赖 BSS=0 的全局 static：

```rust
static STATE: AtomicU8 = AtomicU8::new(STATE_UNINIT);  // 期望 BSS 清零后 = 0
```

但 QEMU virt 的这块 RAM 之前被 OpenSBI 用过，残留了数据。实测 `STATE` 读出来是 `0x10`（非 0），导致 `compare_exchange(0 → 1)` 失败。

**为什么重构前不出问题**：旧的 `board_init` 是空的，不依赖任何 BSS static。driver model 引入 `STATE`/`FDT`/`Slot` 等全局状态后，这个潜藏 bug 才暴露。

### 修复

`platform` 的 `riscv64` feature 连带开启 `riscv64-rt/clear_bss`，在 `__start` 汇编的最早处（设置 mtvec 前）清零 `__sbss..__ebss`：

```rust
// platform/Cargo.toml
[features]
riscv64 = ["dep:riscv64-rt", "riscv64-rt/clear_bss"]
```

```asm
// riscv64-rt/src/start.rs 的 __start
__start:
    la gp, __global_pointer$
    la sp, __sstack
    call __clear_bss      ← 清零 BSS
    call __start_rust
    j __rust_main
```

一处改动，hart0 和 hart1 两条线都受益。修复后实测 `STATE=0x00`，全链路正常。

## 验证：timer async 唤醒链

timer 单中断是 Step 4 的首要验证目标。demo 里新增了 heartbeat 任务：

```rust
#[executor::task]
async fn task_timer_heartbeat() {
    let mut n = 0u32;
    loop {
        futures::timer::after(500.millis()).await;
        n += 1;
        log::info!("[heartbeat] tick #{n}");
    }
}
```

完整的异步唤醒链在 hart1 全链路下验证通过：

```
CLINT timer 中断 (mtimecmp @ base+0x4008, boot_cpuid_phys=1 推导)
  → MachineTimer ISR
  → futures::timer::handle_timer_isr()
  → TimerQueue.dequeue_expired → wake_trampoline → waker.wake_by_ref()
  → wake_task → platform::pend() → 写 MSIP1
  → MachineSoft ISR → clear_pend()=true → try_preempt → run
  → poll task_timer_heartbeat → after(500ms) Ready
  → "[heartbeat] tick #N"
```

实测 8 秒内 tick #1~#15 稳定输出（每 500ms 一次），证明：
- CLINT driver 的 hart1 偏移正确（mtimecmp @ base+0x4008）
- driver model 的 timer registry 转发正常
- async waker 经 TimerQueue 驱动正常
- 整个唤醒链在双核 AMP（OpenSBI + StarryOS hart0 + rt-async hart1）下正常

## 上层零改动是怎么做到的

重构的核心约束：**19 个 bin、宏 codegen、`platform::init` 签名、中断弱符号机制全部不变**。

关键在于 `ChipImpl` / `TimerChipImpl`（`extern_trait` 静态分发）从"实现者"退化为"转发 shim"：

```rust
// 重构前：chip crate 直接操作硬件
impl TimerChip for QemuVirtRt {
    fn set_deadline(tick: u64) {
        unsafe { write_volatile((CLINTBASE + 0x4008) as *mut u64, tick) };
    }
}

// 重构后：chip crate 纯转发到 driver registry
impl TimerChip for QemuVirtRt {
    fn set_deadline(tick: u64) {
        platform::driver::timer().set_deadline(tick)
    }
}
```

上层调用 `TimerChipImpl::set_deadline(t)` 的路径完全不变——`extern_trait` 仍然把调用静态分发给 chip crate 的实现，只是实现体内部从"直接写 MMIO"变成"经 registry 间接写"。executor、futures、宏 codegen 都无需感知 driver model 的存在。

## 代码审阅与修复

完成初版后做了两轮代码审阅（subagent），修复了以下问题：

| 级别 | 问题 | 修复 |
|---|---|---|
| HIGH | `timer freq_hz` 未 probe 时返回 0（注释声称 10MHz）。`after()` 内 `ticks = duration × freq_hz / 1e9`，freq=0 时 ticks=0、deadline=now，future 不等待立刻 Ready（sleep 失效） | FREQ=0 时返回 10_000_000 |
| HIGH | `set_drivers` 三个独立原子量割裂 + unsafe | 改用 `Slot<&[&dyn Driver]>`，统一机制，去掉 unsafe |
| HIGH | `console()` 未注册 panic，panic handler 会二次 panic | 降级返回 `NoOpSerial` |
| MEDIUM | `node_caps[&str;8]` 栈缓冲有截断风险 | 改用内联 `compatibles().any()`，无上限 |
| MEDIUM | 主仓库 3 个 dead_code 警告 | `#[allow(dead_code)]` |
| 注释 | SAFETY 论据循环论证、boot_cpuid_phys 推导未说明 | 改进注释 |

## Phase 2: 去除 Chip/TimerChip，引入 Board trait

### 动机

Step 4 完成后，`Chip`（5 方法：`board_init`/`shutdown`/`put_str`/`pend`/`clear_pend`）和 `TimerChip`（4 方法：`freq_hz`/`now_ticks`/`set_deadline`/`enable_timer_irq`）在 QEMU virt 平台的实现体已经**退化为纯转发 shim**：

```rust
// 两个 chip crate 的实现体里，所有方法都是这样一层转发
fn put_str(s: &str) { platform::driver::console().write(s.as_bytes()); }
fn shutdown() -> ! { platform::driver::reset().shutdown() }
fn pend()          { unsafe { platform::driver::ipi().send() }; }
// ...
```

`extern_trait` 的静态分发优势仍在，但 `Chip`/`TimerChip` 这种"把不相关的功能塞进一个 trait"的设计已经过时。重构目标：**删除这两个 trait，只保留极简 `Board` trait（仅 `fn init()` 一项），把其余方法全部迁移到 driver registry**。

### 迁移映射

| 旧方法 | 新调用方 |
|---|---|
| `ChipImpl::shutdown()` | `platform::reset().shutdown()` |
| `ChipImpl::put_str(s)` | `platform::console().write(s.as_bytes())` |
| `ChipImpl::pend()` | `platform::ipi().send()` |
| `ChipImpl::clear_pend()` | `platform::ipi().clear()` |
| `TimerChipImpl::freq_hz()` | `platform::timer().freq_hz()` |
| `TimerChipImpl::now_ticks()` | `platform::timer().now()` |
| `TimerChipImpl::set_deadline(t)` | `platform::timer().set_deadline(t)` |
| `TimerChipImpl::enable_timer_irq()` | 拆为 `timer().set_deadline(MAX)` + `arch::enable_mtimer()`，在 `platform::start()` 内联 |

### 架构变化

```
重构前                                    重构后
──────                                    ──────
#[extern_trait(pub ChipImpl)]             #[extern_trait(pub BoardImpl)]
pub trait Chip { 5 methods }              pub trait Board { fn init(); }
                                        
#[extern_trait(pub TimerChipImpl)]        (TimerChip 删除)
pub trait TimerChip { 4 methods }         
                                        
chip crate: impl Chip + TimerChip          chip crate: impl Board (仅 init)
  ├─ board_init: DTB + drivers + boot       └─ init: DTB + drivers + boot + register_irq
  ├─ put_str → 裸 MMIO
  ├─ shutdown → 裸 MMIO                   platform::start(): 内联 enable_timer_irq
  └─ ...                                    └─ timer().set_deadline + arch::enable_mtimer
```

`platform::init(log_level)` 签名不变、executor-macro codegen 零改动、`__rust_main` 不动。

---

## Phase 3: 中断分发机制（零抽象开销）

### 三种 RISC-V 中断

rt-async 涉及的中断有三条线，用途和机制各不相同：

| | MachineSoft (MSIP) | MachineTimer (MTimer) | MachineExternal (MEI) |
|---|---|---|---|
| **信号源** | CLINT MSIP 寄存器 | CLINT mtimecmp | PLIC（多源汇聚） |
| **使能** | `mie.MSIE` | `mie.MTIE` (**新增 arch::enable_mtimer**) | `mie.MEIE` |
| **ISR 提供方** | executor-macro 强制强符号 | 用户 `#[executor::interrupt]` | **arch 强符号默认值** → dispatch |
| **需要 register_irq？** | 否（单用途：调度器唤醒） | 否（单用途：定时器队列） | **是（PLIC 多源分发）** |
| **App 侧 API** | 透明（waker 链自动触发） | `TimerDelay::new(t).await` | `SerialRx::new().await` |

### 外部中断分发的零开销设计

PLIC 是多源中断控制器，所有外设（UART、SPI、网卡等）共享一条 MachineExternal 线。分发层的核心是一个**以 IRQ 编号直接索引的静态数组**：

```rust
// platform/src/irq.rs
pub type IrqHandler = unsafe fn(irq: u32);

const MAX_IRQ: usize = 64;  // QEMU virt PLIC 53 源，64 留有余量
static IRQ_TABLE: [AtomicUsize; MAX_IRQ] = [...] ;

pub fn register_irq(irq: u32, handler: IrqHandler) {
    IRQ_TABLE[irq].store(handler as usize, Release);  // 启动期注册
}

pub fn dispatch_external() {
    let irq = intctl().claim();        // PLIC CLAIM（读）
    if irq == 0 { complete(0); return; }
    let handler = IRQ_TABLE[irq]       // O(1) 数组下标 → 函数指针
        .load(Acquire);
    if handler != 0 { handler(irq); }  // 调用 handler（fn 指针，无 vtable）
    intctl().complete(irq);            // PLIC COMPLETE（写同一寄存器）
}
```

**零开销证明**：

| 操作 | 开销 | 说明 |
|---|---|---|
| 查找 | `IRQ_TABLE[irq]` → 一次 Load | O(1)，无哈希、无链表、无排序 |
| 调用 | 函数指针 → RISC-V `jalr` | 无 vtable，与 `dyn Trait` 比省去两次间接 |
| 注册 | `AtomicUsize::store(Release)` | 关中断上下文，单写者，无锁 |
| 内存 | 64 × 8 = 512 字节 | 编译期确定，零堆分配 |

**默认 MachineExternal**：arch crate 提供一个强符号 `__rt_machine_external`，link.x 通过 `PROVIDE(MachineExternal = __rt_machine_external)` 将其设为默认 handler。App **不再需要**手写 `#[executor::interrupt] fn MachineExternal`——只需 `register_irq` 注册即可。若 App 确实需要自定义 MachineExternal，仍可提供同名强符号覆盖（PROVIDE 是弱符号，强符号优先）。

### 完整数据流：从外设中断到任务唤醒

```
UART 发送字节
  → PLIC 判定优先级 + 使能
  → hart1 MachineExternal 中断
  → arch::MachineExternal → irq::dispatch_external()
      irq = intctl().claim()            // PLIC CLAIM 读，例如 irq=12
      handler = IRQ_TABLE[12]           // ns16550a::rx_handler
      handler(12):                      // 调 handler
        while let Some(b) = read_fifo() → ring.push(b)  // UART FIFO → 环形缓冲区
        waker_slot.wake():                              // 唤醒等待任务
          has_waker.swap(false) → waker.wake()
            → wake_task → enqueue → platform::pend()
            → MSIP[hart1] = 1 → MachineSoft → 调度器
              → SerialRx::poll → ring.pop() → Ready(byte)
      intctl().complete(12)             // PLIC COMPLETE 写
```

---

## Phase 4: InterruptController trait + PLIC 驱动

### InterruptController trait

```rust
pub trait InterruptController: Send + Sync {
    fn enable_irq(&self, irq: u32);
    fn disable_irq(&self, irq: u32);
    fn set_priority(&self, irq: u32, prio: u32);
    fn set_threshold(&self, thr: u32);
    fn claim(&self) -> u32;
    fn complete(&self, irq: u32);
}
```

### PLIC 驱动（`drivers/plic_sifive.rs`）

零大小单例 `Plic`，实现 `Driver`（DT 探测）+ `InterruptController`。

```rust
impl Driver for Plic {
    fn compatible(&self) -> &'static [&'static str] { &["riscv,plic0"] }
    fn probe(&self, node: &Node<'_>) {
        let base = node.reg().next().expect("missing reg");
        BASE.store(base.address, Release);
        // context 从当前 hart id 推导。hart0 M-mode = 0, hart1 M-mode = 2。
        let hart_id = riscv::register::mhartid::read();
        CONTEXT.store(hart_id * 2, Release);
        set_intctl(&PLIC);
    }
}
```

**不再硬编码 `context = 2`**：旧 chip crate 的 `mod plic` 手写了 `CONTEXT_BASE + 2*0x80` 等偏移。新驱动从 `mhartid * 2` 动态推导，hart0 和 hart1 共用同一份驱动代码。

### 板级 IRQ 注册

在 `Board::init` 里，`boot()` 实例化全部驱动后，板级负责把 IRQ 号与驱动 handler 绑定：

```rust
impl Board for QemuVirtRt {
    fn init() {
        locate_rtasync_dtb() → init_dtb → set_drivers → boot();
        // boot 已完成，全部驱动就绪
        platform::register_irq(UART1IRQ, ns16550a::rx_handler);
        platform::intctl().enable_irq(UART1IRQ);
        platform::intctl().set_priority(UART1IRQ, 2);
    }
}
```

---

## Phase 5: 异步串口接收（SerialRx Future）

### 驱动内置环形缓冲区

NS16550A 驱动在 `probe` 时使能 FIFO + ERBFI（RX 中断），并在内部维护：

```rust
struct RxState {
    head: AtomicU16,                          // ISR 写索引
    tail: AtomicU16,                          // Task 读索引
    buf: UnsafeCell<[u8; 256]>,              // 环形缓冲区
    has_waker: AtomicBool,                    // Waker 槽占用标记
    waker: UnsafeCell<MaybeUninit<Waker>>,   // Waker 槽
}
unsafe impl Sync for RxState {}
static RX: RxState = ...;
```

### 公开 API

| API | 调用方 | 功能 |
|---|---|---|
| `rx_handler(irq)` | IRQ 分发（ISR 上下文） | 排出 UART FIFO → push ring → wake task |
| `rx_poll(cx)` | `SerialRx::poll`（Task 上下文） | 经典的 disable→register waker→recheck→enable 临界区模式 |
| `read()` → `Option<u8>` | 轮询 App | 读硬件 RBR（不经过 ring） |
| `has_data()` → `bool` | 轮询 App | 读硬件 LSR.DR |
| `write(&[u8])` | 任何上下文 | 逐字节写 THR |

### SerialRx Future

```rust
// futures/serial.rs
pub struct SerialRx;
impl Future for SerialRx {
    type Output = u8;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<u8> {
        platform::drivers::serial_ns16550a::rx_poll(cx)
    }
}
```

**rx_poll 的临界区模式**（与 `uart_wait.rs` 一致）：

1. **快速路径**：ring 非空 → 立即 Pop 返回 Ready
2. **关中断**，注册 cx.waker()
3. **重检 ring**：若 ISR 在注册间隙推入字节 → 取字节、拆回 waker、开中断、返回 Ready
4. **开中断**，返回 Pending

### App 对比

```rust
// ── 重构前（console_interrupt.rs，~230 行）──
#[executor::interrupt]
fn MachineExternal(_tf: &mut TrapFrame) {
    let irq = plic_claim();
    if irq == UART1_IRQ {
        while chip::uart_has_data() {
            let byte = chip::uart_read_byte();
            uart_wait::push_byte(byte);
        }
        uart_wait::notify_from_isr();
    }
    plic_complete(irq);
}
async fn task_console() {
    loop {
        let byte = uart_wait::WaitForByte::new().await;
        // 处理...
    }
}

// ── 重构后（console_interrupt.rs，~50 行）──
async fn task_console() {
    loop {
        let byte = futures::serial::SerialRx::new().await;
        // 处理...
    }
}
```

**无需 MachineExternal ISR、无需环形缓冲区、无需 PLIC 算术、无需 Waker 槽管理。**

---

## 完整启动流程

```
__rust_main (executor-macro 生成)
  → platform::init(log_level)
      → LOGGER.init        ← 注册 log 回调（写 console，此时 console 尚未就绪，log 不输出）
      → arch::arch_init    ← mtvec 等 arch 初始化（已在 __start 中设置）
      → BoardImpl::init()  ← extern_trait 静态分发到板级实现
          │
          │  [子模块 qemu-virt]
          │    init_dtb(include_bytes!("../../its/rt-async-qemu-virt.dtb"))
          │    set_drivers(default_drivers())
          │    boot()
          │
          │  [主仓库 chip-qemu-virt-rt]
          │    locate_rtasync_dtb()   ← esos 同款扫描 0x83000000
          │    init_dtb(dtb)
          │    set_drivers(default_drivers())
          │    boot()
          │      ├─ probe NS16550A  → set_console(&INSTANCE)
          │      ├─ probe PLIC      → set_intctl(&PLIC)
          │      ├─ probe CLINT     → set_timer(&INSTANCE)
          │      ├─ probe MSIP      → set_ipi(&INSTANCE)
          │      └─ probe SiFive    → set_reset(&INSTANCE)
          │    register_irq(UART1IRQ, ns16550a::rx_handler)
          │    intctl().enable_irq(UART1IRQ)
          │    intctl().set_priority(UART1IRQ, 2)
          │
          │  [std-chip (host 单测)]
          │    set_console(&STD_SERIAL)   ← print!
          │    set_timer(&STD_TIMER)      ← stub
          │    set_reset(&STD_RESET)      ← exit(0)
          │    set_ipi(&STD_IPI)          ← no-op
          │
          └── 返回 platform::init，driver registry 全部就绪

  → 用户 async fn main()  ← spawner.run 执行应用代码
  → platform::start()     ← 应用代码返回后
      timer().set_deadline(u64::MAX)  ← 推远定时器截止
      arch::enable_mtimer()           ← 开 MachineTimer 中断
      arch::enable_msi()              ← 开 MachineSoft 中断
      arch::enable_mei()              ← 开 MachineExternal 中断
      arch::enable_interrupts()       ← 开全局中断 (mstatus.MIE)
  → loop { arch::idle() }  ← WFI 等待中断，调度器接管
```

---

## 完整中断分发流程

```
┌─────────────────────────────────────────────────────────────┐
│  RISC-V 中断线                                              │
├──────────────────┬──────────────────┬───────────────────────┤
│  MachineSoft     │  MachineTimer    │  MachineExternal      │
│  (hart MSIP)     │  (CLINT mtimecmp)│  (PLIC → 多源)        │
│                  │                  │                        │
│  executor-macro  │  #[interrupt]    │  arch 强符号默认值    │
│  强制强符号      │  覆盖弱符号       │  __rt_machine_external │
│       ↓          │       ↓          │       ↓                │
│  __Inner_MS      │  handle_timer    │  dispatch_external()  │
│  + clear_pend    │  _isr()          │       ↓                │
│  + 调度器        │       ↓          │  claim()              │
│                  │  timer().now()   │  IRQ_TABLE[irq]       │
│                  │  queue.dequeue   │  handler(irq)         │
│                  │  timer().set_    │  complete()           │
│                  │  deadline()      │                        │
│  ←──── Waker 链 ──→                 │  ←──── Waker 链 ──→  │
│                  │                  │                        │
│  API: 透明       │  API:            │  API:                 │
│  (waker 自动     │  after().await   │  SerialRx::new()     │
│   触发调度器)    │                   │       .await          │
└──────────────────┴──────────────────┴───────────────────────┘
```

---

## 更新后的驱动注册表

```rust
// driver.rs — 全局 Slot<T> 槽位
static CONSOLE: Slot<&'static dyn Serial> = ...;
static TIMER:   Slot<&'static dyn Timer>  = ...;
static IPI:     Slot<&'static dyn Ipi>    = ...;
static RESET:   Slot<&'static dyn Reset>  = ...;
static INTC:    Slot<&'static dyn InterruptController> = ...;  // 新增
```

| 槽位 | 类型 | 驱动 | compatible |
|---|---|---|---|
| CONSOLE | `&dyn Serial` | NS16550A | `ns16550a` |
| TIMER | `&dyn Timer` | CLINT Timer | `riscv,clint0` |
| IPI | `&dyn Ipi` | CLINT MSIP | `riscv,clint0-msip` |
| RESET | `&dyn Reset` | SiFive Test | `sifive,test1` |
| **INTC** (新) | `&dyn InterruptController` | PLIC | `riscv,plic0` |

---

## 驱动组织：平台内置 + Chip 追加

`platform::drivers::default_drivers()` 返回全部 5 个内置驱动。chip crate 可构造自己的 `&'static [&'static dyn Driver]` 切片，在默认列表基础上追加独有驱动后传给 `set_drivers()`。

加新驱动的步骤：

```rust
// 1. drivers/my_driver.rs — impl Driver + 功能 trait
// 2. drivers/mod.rs — pub mod + pub use
// 3. 加入 default_drivers() 数组
```

**无宏、无 link section、无注册函数。** 加一个驱动 = 写 `impl Driver` + 加进数组。

---

## K3 状态

`chip-k3-rt24` 和 `apps/rt-async-k3` **本轮暂移出 workspace**。K3 没有 DTB、没有 PLIC、没有 driver registry，直接用 `clock::early_init()` + `uart::putc` 做裸 MMIO 初始化。移除 Chip trait 后 K3 需要一个 `Board` impl + `Serial` wrapper 来保持编译，但完整的驱动模型适配将在后续轮次完成。

---

## 变更规模

| 轮次 | 子仓库 | 主仓库 |
|---|---|---|
| Phase 1-2 (DTB + driver model + Chip shim) | +796 / -30 行 | +241 / -26 行 |
| Phase 3-5 (Board trait + IRQ dispatch + SerialRX) | +657 / -203 行 | +53 / -250 行 |
| **总计** | **~30 文件变更** | **~8 文件变更** |

---

## 提交记录

```
子仓库 feat/driver-model-dtb:
  b7d1b87  Step1: DTB 解析层 + 子模块内嵌 handoff
  1762977  Step2: driver model + registry + ChipImpl/TimerChipImpl shim
  f9340f1  CLINT timer/ipi hart 感知偏移
  1018f11  fix: riscv64 feature 启用 clear_bss (根因修复)
  0a2a63d  refactor: 代码审阅修复
  34a4631  chore: 简化工程复杂度 (H3 freq_hz 回退 + 删 NoOpSerial + dtb 简化)
  d2d3ede  refactor: 移除 Chip/TimerChip trait + 中断分发 + 异步串口 RX

主仓库 feat/rt-async-driver-model:
  51d3205  Step3: esos 同款 DTB handoff + driver model 接线
  f616eb6  refactor: 代码审阅修复 + 注释改进
  afe7efe  test: timer async heartbeat (Step4 验收)
  9d82e6e  chore: bump rt-async submodule + 移除 K3 工作空间
  93d4ff1  refactor: Board trait + 驱动模型完整化 (中断分发 + 异步串口)
```
