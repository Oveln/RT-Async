//! Driver registry —— driver model 的中枢。
//!
//! 全局槽位持有已实例化的功能设备（console/timer/ipi/reset），由各 driver
//! 的 [`Driver::probe`] 在板级初始化时填充。上层（兼容 shim [`ChipImpl`]/
//! [`TimerChipImpl`]、executor、futures）通过便捷访问器
//! [`console`] / [`timer`] / [`ipi`] / [`reset`] 取用。
//!
//! [`boot`] 遍历设备树，对每个节点按 `compatible` 匹配板级提供的
//! [`DRIVERS`] 列表，命中后调 [`Driver::probe`]。
//!
//! # 设计
//! 所有全局状态都用 [`Slot<T>`] 承载——`MaybeUninit<T>` 配 `AtomicU8` 状态机。
//! `T` 通常是胖指针（`&'static dyn Trait` 或 `&'static [&'static dyn Driver]`），
//! 单个 `usize` 放不下，故用 `MaybeUninit` 承载完整胖指针。`UnsafeCell` 提供
//! 内部可变性（让 `&self` 能在 init 期写入），`Release`/`Acquire` 序保证初始化
//! 结果对后续读者可见。单 hart 串行 probe 场景下安全；多 hart 需保证仅一个
//! hart 调用 `set`。板级 driver 列表（`&'static [&'static dyn Driver]`）同理，
//! 由板级 glue 经 [`set_drivers`] 注入（避免 platform 反向依赖 driver crate）。
//!
//! [`ChipImpl`]: crate::ChipImpl
//! [`TimerChipImpl`]: crate::TimerChipImpl

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;

use fdt_parser::Fdt;
use portable_atomic::{AtomicU8, Ordering};

use crate::device::{Driver, Ipi, Reset, Serial, Timer};

/// 未初始化。
const STATE_UNINIT: u8 = 0;
/// 已就绪。
const STATE_READY: u8 = 1;

/// 一个 `T` 的全局槽位：`UnsafeCell<MaybeUninit>` 承载数据 + 原子状态机。
///
/// `T` 通常是胖指针类型（如 `&'static dyn Serial` 或
/// `&'static [&'static dyn Driver]`）。`MaybeUninit<T>` 承载完整胖指针
/// （数据 + vtable，16 字节），`UnsafeCell` 提供内部可变性（让 `&self`
/// 能在 init 期写入），`AtomicU8` 状态机保证初始化结果对读者可见。
/// 单 hart 串行 init 场景下安全；多 hart 需保证仅一个 hart 调用 `set`。
struct Slot<T> {
    state: AtomicU8,
    val: UnsafeCell<MaybeUninit<T>>,
}

// SAFETY: Slot 的并发安全靠 AtomicU8 状态机保证：单 hart 串行 set，之后只读。
// UnsafeCell 的写只发生在 STATE_UNINIT→READY 转换期（单写者）。
unsafe impl<T: Send> Sync for Slot<T> {}

impl<T> Slot<T> {
    const fn new() -> Self {
        Self {
            state: AtomicU8::new(STATE_UNINIT),
            val: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    fn set(&self, dev: T) {
        // SAFETY: 写入 MaybeUninit。单 hart 串行 init 下无并发写；状态机
        // 用 Release 发布，确保胖指针对后续 Acquire 读者可见。UnsafeCell 提供
        // &self → *mut 的内部可变性。
        unsafe {
            let p: *mut MaybeUninit<T> = self.val.get();
            (*p).write(dev);
        }
        self.state.store(STATE_READY, Ordering::Release);
    }

    fn get(&self) -> Option<&T> {
        if self.state.load(Ordering::Acquire) != STATE_READY {
            return None;
        }
        // SAFETY: STATE == READY 保证值已被写入且对所有读者可见。返回 &T 引用。
        unsafe {
            let p: *const MaybeUninit<T> = self.val.get();
            Some((*p).assume_init_ref())
        }
    }
}

/// 默认 console（由 `chosen { stdout-path }` 选定）。
static CONSOLE: Slot<&'static dyn Serial> = Slot::new();
/// 默认定时器（由 `chosen { timer }` 或首个 Timer 设备选定）。
static TIMER: Slot<&'static dyn Timer> = Slot::new();
/// IPI 设备。
static IPI: Slot<&'static dyn Ipi> = Slot::new();
/// 复位/关机设备。
static RESET: Slot<&'static dyn Reset> = Slot::new();
/// 板级提供的 driver 列表（`&'static [&'static dyn Driver]` 是胖指针）。
static DRIVERS: Slot<&'static [&'static dyn Driver]> = Slot::new();

/// 注册 console 设备。由 Serial driver 的 probe 调用。
pub fn set_console(dev: &'static dyn Serial) {
    CONSOLE.set(dev);
}

/// 注册 timer 设备。由 Timer driver 的 probe 调用。
pub fn set_timer(dev: &'static dyn Timer) {
    TIMER.set(dev);
}

/// 注册 IPI 设备。由 Ipi driver 的 probe 调用。
pub fn set_ipi(dev: &'static dyn Ipi) {
    IPI.set(dev);
}

/// 注册 reset 设备。由 Reset driver 的 probe 调用。
pub fn set_reset(dev: &'static dyn Reset) {
    RESET.set(dev);
}

/// 取默认 console。
///
/// 若未注册（boot 未跑完或 Serial driver probe 失败），返回一个静默丢弃的
/// fallback（no-op）。这样 panic handler 等早期路径在 console 缺失时不会因
/// 二次 panic 把日志通路打死。上层正常使用时 console 已 probe，fallback 不触发。
pub fn console() -> &'static dyn Serial {
    match CONSOLE.get() {
        Some(c) => *c,
        None => &NOOP_SERIAL,
    }
}

/// Fallback console：未 probe 时静默丢弃，避免 panic → put_str → console() 二次 panic。
struct NoOpSerial;
impl Serial for NoOpSerial {
    fn write(&self, _buf: &[u8]) {}
}
static NOOP_SERIAL: NoOpSerial = NoOpSerial;

/// 取默认 timer。若未注册则 panic（timer 不可缺，无静默降级语义）。
pub fn timer() -> &'static dyn Timer {
    *TIMER.get().expect("timer: no Timer device registered")
}

/// 取 IPI 设备。若未注册则 panic。
pub fn ipi() -> &'static dyn Ipi {
    *IPI.get().expect("ipi: no Ipi device registered")
}

/// 取 reset 设备。若未注册则 panic。
pub fn reset() -> &'static dyn Reset {
    *RESET.get().expect("reset: no Reset device registered")
}

/// 设置板级 driver 列表。由板级 glue 在 `board_init` 早期调用，`boot()` 之前。
///
/// `drivers` 必须是 `'static` 有效切片，且仅调用一次。单 hart 串行模型下，
/// `boot()` 在本函数返回后才运行，故胖指针的发布由 `Slot` 的 Release 序保证。
pub fn set_drivers(drivers: &'static [&'static dyn Driver]) {
    DRIVERS.set(drivers);
}

/// 遍历设备树实例化所有 driver。
///
/// 在 `init_dtb` 之后、调度器启动之前由板级 `board_init` 调用。
/// 对每个 DT 节点，按 `compatible` 匹配 [`set_drivers`] 注入的 driver，
/// 命中则调 [`Driver::probe`]。
///
/// # Panics
/// 若 [`set_drivers`] 未调用则 panic。
pub fn boot() {
    let drivers: &&[&dyn Driver] = DRIVERS
        .get()
        .expect("driver::boot: set_drivers() not called");

    let fdt: &Fdt<'static> = crate::dtb::dt();

    for node in fdt.all_nodes() {
        // 对每个 driver 检查节点的 compatible 列表是否有任一命中。
        // node.compatibles() 返回的迭代器每次调用都从头开始，无需收集到栈缓冲，
        // 也无 compatible 个数上限。
        for drv in *drivers {
            let matched = node
                .compatibles()
                .any(|nc| drv.compatible().iter().any(|dc| *dc == nc));
            if matched {
                drv.probe(&node);
            }
        }
    }
}
