//! Driver 与功能 trait 定义。
//!
//! 这是 rt-async driver model 的契约层。每个可被设备树实例化的驱动实现
//! [`Driver`]；具体功能由一组扁平、可选、组合的 trait（[`Serial`]/
//! [`Timer`]/[`Ipi`]/[`Reset`]）表达。板级初始化时，[`Driver::probe`] 从
//! 设备树节点读 reg/irq 等信息，实例化设备并注册进全局 registry
//! （见 [`crate::driver`]），随后通过 [`crate::console`] / [`crate::timer`]
//! 等便捷访问器取用。
//!
//! 设计要点：
//! - **`Driver` 是"可被 DT 探测"的统一入口**，功能 trait 是"这个设备能做什么"。
//!   一个设备可同时实现多个功能 trait（例如 CLINT 同时 impl Timer + Ipi）。
//! - trait 方法全部 `&self`（实例方法），支持同类型多设备实例。
//! - 不引入 async（async waker 在 Step 3 接入；本步先把同步 driver model 立起来）。

use fdt_parser::Node;

/// 设备树节点探测入口。
///
/// 板级 `boot()` 遍历 DT，对每个节点按 `compatible` 匹配已注册 driver，
/// 命中后调用 `probe()`。probe 从 `node` 读 reg/irq 等信息，实例化设备
/// 并注册进 registry（console/timer/ipi/reset 槽位）。
///
/// 实现侧典型形态：
/// ```ignore
/// impl Driver for Ns16550a {
///     fn compatible() -> &'static [&'static str] { &["ns16550a"] }
///     fn probe(node: &Node<'_>) {
///         let reg = node.reg()...;        // 从 DT 读 reg
///         // 实例化 + 注册进 CONSOLE 槽位
///     }
/// }
/// ```
pub trait Driver: Send + Sync {
    /// 该驱动匹配的 compatible 字符串列表（DT 节点的 `compatible` 属性任一命中即匹配）。
    ///
    /// 返回 `&'static` 切片，内容与实例无关；`&self` 仅为对象安全（registry
    /// 用 `&[&dyn Driver]` 并经 trait 对象调用）。driver 实例是零大小单例，
    /// 实现侧忽略 `self`。
    fn compatible(&self) -> &'static [&'static str];

    /// 从设备树节点实例化设备并注册进 registry。
    ///
    /// 节点已由 registry 确认 compatible 匹配。probe 失败属致命错误，
    /// 直接 panic（板级描述与驱动不匹配）。
    ///
    /// `&self` 使 trait 对象安全；driver 实例是零大小单例，`&self` 无运行时
    /// 开销，probe 内部通过全局 `AtomicUsize` 落地 MMIO 基址，不依赖 `self`
    /// 携带数据。
    fn probe(&self, node: &Node<'_>);
}

/// 串口（console）。
pub trait Serial: Send + Sync {
    /// 阻塞写出一串字节。
    fn write(&self, buf: &[u8]);
    /// 从接收 FIFO 读一个字节。若无数据则返回 `None`。
    fn read(&self) -> Option<u8> {
        None
    }
    /// 接收 FIFO 中是否有数据。
    fn has_data(&self) -> bool {
        self.read().is_some()
    }
    /// 中断驱动 RX 的 poll 原语。
    ///
    /// 由 async `SerialRx` Future 经 registry 调用，实现"关中断 → 注册 waker
    /// → 重检 → 开中断"的 ISR/task 竞争修复模式。驱动返回 [`SerialRxStatus`]
    /// 表达当前状态。
    ///
    /// 默认返回 [`SerialRxStatus::Unsupported`]——不支持中断驱动 RX 的驱动
    /// （含 host 桩）静默降级，调用方应回退轮询。
    ///
    /// 该方法与 [`SerialRxStatus`] **不**做 `#[cfg]` 门控：它们始终存在于
    /// trait/enum 中，避免跨 crate（futures）传递 `riscv64` feature。
    /// 不支持中断驱动 RX 的驱动（含 host 桩）依赖默认实现返回 `Unsupported`；
    /// 仅在具体驱动的 override 内部用 cfg 门控 arch 专有逻辑（关/开中断）。
    fn rx_register_waker(&self, _cx: &mut core::task::Context<'_>) -> SerialRxStatus {
        SerialRxStatus::Unsupported
    }
}

/// [`Serial::rx_register_waker`] 的返回状态。
pub enum SerialRxStatus {
    /// 已就绪一个字节。
    Ready(u8),
    /// 已注册 waker，等待 ISR 唤醒；调用方应返回 `Poll::Pending`。
    Pending,
    /// 该驱动不支持中断驱动 RX（默认）；调用方应回退轮询。
    Unsupported,
}

/// pinctrl 控制器功能。
///
/// controller driver 实现，经 probe 注册进 [`crate::driver::PINCTRL`] 全局槽。
/// [`crate::driver::boot`] 遍历 DT 时，对每个节点在 driver probe 之前调用
/// [`PinController::apply`]，使外设引脚配置在驱动看到硬件前就绪。
///
/// 实现侧解析节点的 `pinctrl-0` 属性（phandle → `_cfg` 子节点 →
/// `pinctrl-single,pins` 的 (offset, value) 对），逐对写入 pinmux 寄存器。
/// 无 `pinctrl-0` 的节点调用为 no-op。
pub trait PinController: Send + Sync {
    /// 为给定外设节点应用其 `pinctrl-0` 引脚配置。
    fn apply(&self, node: &Node<'_>);
}

/// 时钟控制器功能（板级 CCU 实现）。
///
/// controller driver 实现，经 probe 注册进 [`crate::driver::CLOCK`] 全局槽。
/// [`crate::driver::boot`] 遍历 DT 时，对每个节点在 driver probe 之前、
/// 应用 pinctrl-0 之后调用 [`ClockProvider::enable_for`]，使外设功能时钟
/// 与复位释放在驱动看到硬件前就绪。
///
/// 实现侧解析节点的 `clocks` 属性，写对应的 gate/mux/div 寄存器并释放
/// 外设复位。无 clocks 配置的节点调用为 no-op。
///
/// 这是 consumer 语义的抽象：trait 只表达"为节点使能时钟"，寄存器细节
/// 留在板级实现（K3 写 RCPU CCU 寄存器，QEMU virt 不注册，boot 容错跳过）。
pub trait ClockProvider: Send + Sync {
    /// 为给定外设节点使能功能时钟并释放复位。无 clocks 属性则 no-op。
    fn enable_for(&self, node: &Node<'_>);
}

/// I2C 总线控制器功能。
///
/// controller driver 实现，经 [`crate::bus`] 注册进 `I2C_BUSES`；
/// child device（eeprom/传感器）经 [`crate::bus::current_i2c`] 取所属 bus
/// 实例收发，不直接 MMIO。对标 Linux `i2c_adapter` / `i2c_client`。
// 消费方：未来 i2c/spi controller + child driver。
#[allow(dead_code)]
pub trait I2cBus: Send + Sync {
    /// 在 `addr` 上执行一次 I2C 传输（可能含多段读写）。
    fn transfer(&self, addr: u8, msg: &mut [I2cMsg<'_>]) -> Result<(), I2cError>;
}

/// I2C 传输的一段（读或写）。
// 消费方：未来 i2c/spi controller + child driver。
#[allow(dead_code)]
pub enum I2cMsg<'a> {
    /// 主机→从机写。
    Write(&'a [u8]),
    /// 从机→主机读（缓冲区在传输中被填充）。
    Read(&'a mut [u8]),
}

/// I2C 传输错误。无具体硬件时仅占位。
// 消费方：未来 i2c/spi controller + child driver。
#[allow(dead_code)]
pub struct I2cError;

/// SPI 总线控制器功能（与 [`I2cBus`] 同构）。
///
/// controller driver 实现并注册；child device 经
/// [`crate::bus::current_spi`] 取 bus 实例收发。对标 Linux `spi_controller`。
// 消费方：未来 i2c/spi controller + child driver。
#[allow(dead_code)]
pub trait SpiBus: Send + Sync {
    /// 全双工 SPI 传输：`write` 的字节同时钟移出，移入的字节填入 `read`。
    /// `read` 与 `write` 等长；较短的一方补零/忽略。
    fn transfer(&self, write: &[u8], read: &mut [u8]) -> Result<(), SpiError>;
}

/// SPI 传输错误。无具体硬件时仅占位。
// 消费方：未来 i2c/spi controller + child driver。
#[allow(dead_code)]
pub struct SpiError;

/// 中断控制器（Platform-Level Interrupt Controller / 核内中断路由）。
///
/// 外设通过 PLIC 等中断控制器汇总到 hart 的 Machine External 中断线。
/// 该 trait 封装 claim / complete / enable 等操作，供中断分发层
/// ([`crate::irq::dispatch_external`]) 和各驱动使用。
pub trait InterruptController: Send + Sync {
    /// 在中断控制器中使能指定外设中断源。
    fn enable_irq(&self, irq: u32);
    /// 禁能指定外设中断源。
    fn disable_irq(&self, irq: u32);
    /// 设置中断源优先级（0 = 禁能，通常 1–7）。
    fn set_priority(&self, irq: u32, prio: u32);
    /// 设置中断控制器优先级阈值。
    fn set_threshold(&self, thr: u32);
    /// 认领当前触发的中断源 ID（读 PLIC claim 寄存器）。
    /// 返回 0 表示虚假中断。
    fn claim(&self) -> u32;
    /// 标记中断处理完成（写 PLIC claim/complete 寄存器）。
    fn complete(&self, irq: u32);
}

/// 硬件定时器（单调时钟 + 单次截止时间）。
pub trait Timer: Send + Sync {
    /// 时钟频率（Hz）。
    fn freq_hz(&self) -> u32;
    /// 当前 tick 计数（单调递增）。
    fn now(&self) -> u64;
    /// 设置下一次定时器中断的截止 tick。
    fn set_deadline(&self, tick: u64);
}

/// 核间中断（IPI）。
///
/// rt-async 的 executor 用 IPI 触发抢占重调度。`send` 写 IPI 寄存器，
/// `clear` 在 ISR 里清除 pending。
pub trait Ipi: Send + Sync {
    /// 发出 IPI。
    ///
    /// # Safety
    /// 触发中断、依赖具体硬件 IPI 机制，调用者须保证在中断上下文外、
    /// 且调度器已就绪。
    unsafe fn send(&self);

    /// 清除 IPI pending。在 ISR 早期调用。
    ///
    /// # Safety
    /// 通常在关中断的 ISR 上下文调用。
    unsafe fn clear(&self);
}

/// 复位/关机。
pub trait Reset: Send + Sync {
    /// 关机（不掉电则死循环）。永不返回。
    fn shutdown(&self) -> !;
}
