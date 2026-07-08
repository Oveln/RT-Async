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
//! - `&'static dyn Trait` 是胖指针（数据指针 + vtable 指针），单个 `usize`
//!   放不下。这里用 `MaybeUninit<&'static dyn Trait>` 承载完整胖指针，配
//!   `AtomicU8` 状态机（参照 [`crate::dtb`]）。单 hart 串行 probe 场景下
//!   安全；`Release`/`Acquire` 序保证初始化结果对后续读者可见。
//! - 板级 driver 列表 `DRIVERS` 同理（`&'static [&'static dyn Driver]` 是胖指针），
//!   由板级 glue 经 [`set_drivers`] 注入（避免 platform 反向依赖 driver crate）。
//!
//! [`ChipImpl`]: crate::ChipImpl
//! [`TimerChipImpl`]: crate::TimerChipImpl

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;

use fdt_parser::Fdt;
use portable_atomic::{AtomicU8, AtomicUsize, Ordering};

use crate::device::{Driver, Ipi, Reset, Serial, Timer};

/// 未初始化。
const STATE_UNINIT: u8 = 0;
/// 已就绪。
const STATE_READY: u8 = 1;

/// 一个 `&'static dyn Trait` 槽位：`UnsafeCell<MaybeUninit>` 承载胖引用 +
/// 原子状态机。
///
/// `T` 是引用类型本身（如 `&'static dyn Serial`）。`MaybeUninit<T>` 承载完整
/// 胖指针（数据 + vtable，16 字节），`UnsafeCell` 提供内部可变性（让 `&self`
/// 能写入），`AtomicU8` 状态机保证初始化结果对读者可见。单 hart 串行 probe
/// 场景下安全；多 hart 需保证仅一个 hart 调用 `set`。
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
        // SAFETY: 写入 MaybeUninit。单 hart 串行 probe 下无并发写；状态机
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

/// 板级提供的 driver 列表。`&'static [&'static dyn Driver]` 是胖指针，
/// 同样用 MaybeUninit + 状态机承载。
static DRIVERS_PTR: AtomicUsize = AtomicUsize::new(0);
static DRIVERS_LEN: AtomicUsize = AtomicUsize::new(0);
static DRIVERS_READY: AtomicU8 = AtomicU8::new(STATE_UNINIT);

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

/// 取默认 console。若未注册则 panic（console 不可缺）。
pub fn console() -> &'static dyn Serial {
    *CONSOLE
        .get()
        .expect("console: no Serial device registered")
}

/// 取默认 timer。若未注册则 panic。
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

/// 设置板级 driver 列表。由板级 glue 在 `board_init` 早期调用。
///
/// # Safety
/// `drivers` 必须是 'static 有效引用，且仅调用一次（boot 前）。
pub unsafe fn set_drivers(drivers: &'static [&'static dyn Driver]) {
    // 切片引用是胖指针（data + len）。拆成 data 指针 + len 两个原子存。
    DRIVERS_PTR.store(drivers.as_ptr() as usize, Ordering::Release);
    DRIVERS_LEN.store(drivers.len(), Ordering::Release);
    DRIVERS_READY.store(STATE_READY, Ordering::Release);
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
    if DRIVERS_READY.load(Ordering::Acquire) != STATE_READY {
        panic!("driver::boot: set_drivers() not called");
    }
    let ptr = DRIVERS_PTR.load(Ordering::Acquire) as *const &dyn Driver;
    let len = DRIVERS_LEN.load(Ordering::Acquire);
    // SAFETY: 由 set_drivers 写入，源自 'static 切片引用；data 与 len 都对齐可见。
    let drivers: &[&dyn Driver] = unsafe { core::slice::from_raw_parts(ptr, len) };

    let fdt: &Fdt<'static> = crate::dtb::dt();

    for node in fdt.all_nodes() {
        // 节点的 compatible 可能多个，driver 的 compatible 列表也可能多个；
        // 任一命中即 probe。先收集节点 compatible 到栈上小缓冲避免迭代器 Clone。
        let mut node_caps: [&str; 8] = [""; 8];
        let mut count = 0usize;
        for nc in node.compatibles() {
            if count < node_caps.len() {
                node_caps[count] = nc;
                count += 1;
            }
        }
        if count == 0 {
            continue;
        }
        let node_caps = &node_caps[..count];

        for drv in drivers {
            let drv_compatibles = drv.compatible();
            let matched = node_caps
                .iter()
                .any(|nc| drv_compatibles.iter().any(|dc| *dc == *nc));
            if matched {
                drv.probe(&node);
            }
        }
    }
}
