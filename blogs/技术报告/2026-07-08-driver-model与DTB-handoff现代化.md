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

## 后续工作

当前 driver model 已覆盖 Serial/Timer/Ipi/Reset 四类设备。计划中的 Step 4 后续是 **dispatch_irq 外部中断路由**（PLIC IrqController driver）：

- 引入 `IrqController` trait（claim/complete/enable）
- PLIC 抽象为 driver，DT probe 时读 `interrupts` 属性建立 IRQ→设备绑定
- `MachineExternal` 弱符号内填通用骨架：`claim → dispatch_irq(irq_id) → complete`
- 统一 timer 的回调式（TimerQueue）与未来的设备 IRQ waker 式唤醒

这部分是独立工作量，作为后续轮次。

## 提交记录

```
子模块 feat/driver-model-dtb:
  b7d1b87  Step1: DTB 解析层 + 子模块内嵌 handoff
  1762977  Step2: driver model + registry + ChipImpl/TimerChipImpl shim
  f9340f1  CLINT timer/ipi hart 感知偏移
  1018f11  fix: riscv64 feature 启用 clear_bss (根因修复)
  0a2a63d  refactor: 代码审阅修复

主仓库 feat/rt-async-driver-model:
  51d3205  Step3: esos 同款 DTB handoff + driver model 接线
  f616eb6  refactor: 代码审阅修复 + 注释改进
  afe7efe  test: timer async heartbeat (Step4 验收)
```
