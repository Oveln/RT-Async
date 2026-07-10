//! Driver registry —— driver model 的中枢。
//!
//! 全局槽位持有已实例化的功能设备（console/timer/ipi/reset/intctl），由各 driver
//! 的 [`Driver::probe`] 在板级初始化时填充。上层（executor、futures）通过便捷访问器
//! [`console`] / [`timer`] / [`ipi`] / [`reset`] / [`intctl`] 取用。
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
//! hart 调用 `set`。
//!
//! driver 注入采用直写式：各 driver 的 probe 直接调用 [`CONSOLE::set`] /
//! [`TIMER::set`] 等公开槽位；板级 driver 列表（`&'static [&'static dyn Driver]`）
//! 由板级 glue 经 [`DRIVERS::set`] 注入（避免 platform 反向依赖 driver crate）。
//!
//! 同类型多实例设备（串口、I2C/SPI bus）用 [`DeviceRegistry<T, N>`]——它构建在
//! `[`Slot`]` 数组之上，配线性探测游标，`register` 找首个空槽填入并返回索引。

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;

use fdt_parser::Fdt;
use portable_atomic::{AtomicU8, AtomicUsize, Ordering};

use crate::device::{Driver, InterruptController, Ipi, PinController, Reset, Serial, Timer};

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
pub struct Slot<T> {
    state: AtomicU8,
    val: UnsafeCell<MaybeUninit<T>>,
}

// SAFETY: Slot 的并发安全靠 AtomicU8 状态机保证：单 hart 串行 set，之后只读。
// UnsafeCell 的写只发生在 STATE_UNINIT→READY 转换期（单写者）。
unsafe impl<T: Send> Sync for Slot<T> {}

impl<T> Slot<T> {
    pub const fn new() -> Self {
        Self {
            state: AtomicU8::new(STATE_UNINIT),
            val: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    pub fn set(&self, dev: T) {
        // SAFETY: 写入 MaybeUninit。单 hart 串行 init 下无并发写；状态机
        // 用 Release 发布，确保胖指针对后续 Acquire 读者可见。UnsafeCell 提供
        // &self → *mut 的内部可变性。
        unsafe {
            let p: *mut MaybeUninit<T> = self.val.get();
            (*p).write(dev);
        }
        self.state.store(STATE_READY, Ordering::Release);
    }

    pub fn get(&self) -> Option<&T> {
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

/// 固定容量的设备注册表：同类型多设备的枚举集合。
///
/// 构建在 `[`Slot`]` 数组之上，配一个线性探测游标。用于同类型多实例设备
/// （串口、I2C/SPI bus）。`register` 找首个空槽填入，返回分配的索引；
/// 满则 panic（与 `IRQ_TABLE`、`heapless::Vec::push().unwrap()` 同语义）。
///
/// `const` 可构造，no_std/no-alloc，单 hart 串行 init 安全。
pub struct DeviceRegistry<T, const N: usize> {
    slots: [Slot<T>; N],
    next: AtomicUsize,
}

impl<T, const N: usize> DeviceRegistry<T, N> {
    /// 创建空注册表。
    pub const fn new() -> Self {
        Self {
            slots: [const { Slot::new() }; N],
            next: AtomicUsize::new(0),
        }
    }

    /// 注册一个设备，返回分配的索引。
    ///
    /// 从游标 `next` 起线性探测首个空槽。单 hart 串行 init 下无并发；
    /// 满（探测 N 个槽均非空）则 panic。返回的索引供 `get` 取回。
    pub fn register(&self, dev: T) -> usize {
        let start = self.next.load(Ordering::Relaxed);
        for i in 0..N {
            let idx = (start + i) % N;
            if self.slots[idx].get().is_none() {
                self.slots[idx].set(dev);
                self.next.store((idx + 1) % N, Ordering::Relaxed);
                return idx;
            }
        }
        panic!("DeviceRegistry::register: capacity {} exhausted", N);
    }

    /// 按索引取设备引用。索引越界或空槽返回 `None`。
    pub fn get(&self, idx: usize) -> Option<&T> {
        let idx = idx.checked_rem(N)?;
        self.slots[idx].get()
    }

    /// 迭代所有已注册设备（跳过空槽）。
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.slots.iter().filter_map(|s| s.get())
    }
}

/// 默认 console（由 `chosen { stdout-path }` 选定）。
pub static CONSOLE: Slot<&'static dyn Serial> = Slot::new();
/// 默认定时器（由 `chosen { timer }` 或首个 Timer 设备选定）。
pub static TIMER: Slot<&'static dyn Timer> = Slot::new();
/// IPI 设备。
pub static IPI: Slot<&'static dyn Ipi> = Slot::new();
/// 复位/关机设备。
pub static RESET: Slot<&'static dyn Reset> = Slot::new();
/// 中断控制器（PLIC 等）。
pub static INTC: Slot<&'static dyn InterruptController> = Slot::new();
/// 板级提供的 driver 列表（`&'static [&'static dyn Driver]` 是胖指针）。
pub static DRIVERS: Slot<&'static [&'static dyn Driver]> = Slot::new();
/// pinctrl 控制器（pinctrl-single 等）。板级 pinctrl driver 在 probe 中
/// `PINCTRL.set(...)` 注入。boot() 遍历 DT 时对每个外设节点在 probe 前调
/// [`PinController::apply`] 应用 `pinctrl-0` 引脚配置。
pub static PINCTRL: Slot<&'static dyn PinController> = Slot::new();

/// 串口设备注册表（多实例）。driver 的 probe 经 [`DeviceRegistry::register`] 登记，
/// `try_derive_console` 据 chosen 从中选出默认 console。
///
/// 单串口板（如 qemu-virt）仅一项；多串口板按 `chosen.stdout-path` 选定。
pub static SERIALS: DeviceRegistry<&'static dyn Serial, 4> = DeviceRegistry::new();

/// 取默认 console。若未注册则 panic（与 timer/ipi/reset 一致）。
///
/// 注意：`boot()` 早期 console 可能尚未就绪（由 `try_derive_console` 在 boot
/// 的 probe 循环内、首个 Serial probe 后据 `chosen.stdout-path` 派生）。boot
/// 期间需输出日志的代码（如 logger、panic handler）应使用 [`try_console`]
/// 容错取用，而非本函数。
pub fn console() -> &'static dyn Serial {
    *CONSOLE
        .get()
        .expect("console: no Serial device registered")
}

/// 取默认 console，未就绪时返回 `None`（不 panic）。
///
/// 供 [`crate::logger`] 等 boot 早期路径使用——此时 console 可能尚未由
/// `try_derive_console` 派生。运行期（调度器启动后）console 必已就绪，
/// 应用代码宜直接用 [`console`]。
pub fn try_console() -> Option<&'static dyn Serial> {
    CONSOLE.get().copied()
}

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

/// 取中断控制器。若未注册则 panic。
pub fn intctl() -> &'static dyn InterruptController {
    *INTC
        .get()
        .expect("intctl: no InterruptController device registered")
}

/// 取 pinctrl 控制器，未就绪时返回 `None`（不 panic）。
///
/// 供 [`boot`] 在 DFS 循环中调用——pinctrl 节点先于外设节点被 probe
/// （DFS 先序），外设 probe 时 pinctrl 已就绪；但 pinctrl controller 自身
/// probe 时 `try_pinctrl()` 返回 `None`（它就是自己在被 probe）。
pub fn try_pinctrl() -> Option<&'static dyn PinController> {
    PINCTRL.get().copied()
}

/// 遍历设备树实例化所有 driver。
///
/// 在 `init_dtb` 之后、调度器启动之前由板级 `board_init` 调用。
/// 对每个 DT 节点，按 `compatible` 匹配 [`DRIVERS`] 槽注入的 driver，
/// 命中则调 [`Driver::probe`]。
///
/// 遍历维护深度感知的 bus 栈（见 [`crate::bus`]）：`all_nodes()` 按 DFS 先序
/// （父先于子）产出节点，`node.level` 反映深度。进入更深的节点前，controller
/// 已被 probe 并 push 进 bus 栈；当 `node.level` 回退（离开某 controller 子树）
/// 时调 [`crate::bus::bus_stack_pop_to`] 清栈，保证后续 child probe 取到正确的
/// 父 bus（或回到根时无活跃 bus）。
///
/// # Panics
/// 若 [`DRIVERS`] 未填充则 panic。
pub fn boot() {
    let drivers: &&[&dyn Driver] = DRIVERS
        .get()
        .expect("driver::boot: DRIVERS not set");

    let fdt: &Fdt<'static> = crate::dtb::dt();

    crate::bus::bus_stack_reset();
    let mut prev_level: usize = 1usize;

    for node in fdt.all_nodes() {
        // 深度回退：离开了之前 controller 的子树，弹出更深的 bus 索引。
        if node.level < prev_level {
            crate::bus::bus_stack_pop_to(node.level);
        }
        // 应用 pinctrl-0：在外设 driver probe 之前配置引脚。
        // pinctrl controller 节点（compatible = "pinctrl-single"）先于外设
        // 节点被 probe（DFS 先序），此时 PINCTRL slot 已就绪；无 pinctrl-0
        // 的节点（含 pinctrl controller 自身）apply 为 no-op。
        if let Some(pc) = try_pinctrl() {
            pc.apply(&node);
        }
        // 对每个 driver 检查节点的 compatible 列表是否有任一命中。
        // node.compatibles() 迭代器每次调用都从头开始，无需收集到栈缓冲，
        // 也无 compatible 个数上限。
        for drv in *drivers {
            let matched = node
                .compatibles()
                .any(|nc| drv.compatible().iter().any(|dc| *dc == nc));
            if matched {
                drv.probe(&node);
            }
        }
        // 派生 console：首个 Serial 设备 probe 后立即派生（而非等到 boot 末尾），
        // 使后续 probe（如 PLIC 的 log::info）能经 logger 正常输出——logger 在
        // console 未就绪时虽经 [`try_console`] 容错不 panic，但会丢弃日志。
        // DT 中 serial 节点先于 plic（DFS 先序），故此处保证 PLIC probe 前
        // console 已就绪。CONSOLE 已派生后此调用为幂等 no-op。
        if CONSOLE.get().is_none() {
            try_derive_console(fdt);
        }
        prev_level = node.level;
    }

    // 兜底：确保 boot 返回时 console 已派生（如 DTS 无 chosen 或无 serial）。
    try_derive_console(fdt);
}

/// 从设备树 `chosen { stdout-path }` 派生默认 console。
///
/// 在 [`boot`] 的 probe 循环中，首个 Serial 设备 probe 后即调用（而非等到
/// boot 末尾），保证后续 probe 的日志（如 PLIC 的 `log::info`）能正常输出。
/// `fdt_parser` 已处理 alias/路径/`"name:baud"` 后缀。
///
/// **不 panic**：若无 Serial 设备被 probe（如纯 timer 板），则静默返回——
/// console 非所有板的必需项，缺失时 [`logger`] 经 [`try_console`] 容错。
/// 真正缺 console 的板，运行期调 [`console`] 时才 panic。
///
/// 当前为单串口板简化版：`SERIALS` 仅一项时直接提升为 console；
/// `chosen.stdout` 仅作「期望有 console」的校验。多串口按 `stdout.node.name`
/// 在 SERIALS 中匹配留待未来（需 driver 记录节点名）。
///
/// `std-chip`（host 桩）不经此路径——它无 DTB、不调 `boot()`，直接
/// `CONSOLE.set(...)`。
fn try_derive_console(fdt: &Fdt<'static>) {
    let has_chosen = fdt.chosen().and_then(|c| c.stdout()).is_some();
    // 无 Serial 被 probe：console 非所有板的必需项，静默返回。
    // 但若 chosen.stdout-path 已设却无 serial 驱动匹配，记一条警告——
    // 否则该配置错误只会在运行期首次 console() 时以笼统的 panic 暴露。
    let Some(dev) = SERIALS.iter().next().copied() else {
        if has_chosen {
            log::warn!("chosen.stdout-path set but no serial driver matched");
        }
        return;
    };
    // 注：单串口板下 chosen 仅作「期望有 console」的校验；多串口按
    // stdout.node.name 在 SERIALS 中匹配留待未来（需 driver 记录节点名）。
    CONSOLE.set(dev);
}
