//! Device Tree Blob (DTB) 解析层。
//!
//! 板级初始化时调用 [`init_dtb`] 注入 DTB 切片（来源由板级决定：
//! 子模块内嵌 / 主仓库 esos 同款扫描 / 等），之后 [`dt`] 返回全局句柄供
//! driver probe 使用。
//!
//! 实现说明：`core::cell::OnceCell` 不是 `Sync`，无法直接用作裸机 `static`。
//! 这里用 `portable_atomic::AtomicU8` 状态机 + `MaybeUninit` 承载数据，
//! 借助 `Acquire`/`Release` 序保证初始化结果对后续读取可见。单 hart 串行
//! probe 场景下安全；多 hart 需保证只有一个 hart 调用 [`init_dtb`]。

use core::mem::MaybeUninit;
use core::ptr::addr_of_mut;

use fdt_parser::Fdt;
use portable_atomic::{AtomicU8, Ordering};

/// 未初始化。
const STATE_UNINIT: u8 = 0;
/// 正在初始化（占位，单 hart 下不会真正用到，但保留以备多 hart 扩展）。
const STATE_INITING: u8 = 1;
/// 已就绪。
const STATE_READY: u8 = 2;

static STATE: AtomicU8 = AtomicU8::new(STATE_UNINIT);
static mut FDT: MaybeUninit<Fdt<'static>> = MaybeUninit::uninit();

/// 注入 DTB 切片。由板级 `board_init` 调用，来源不限（内嵌/扫描/handoff）。
///
/// # Panics
/// `dtb` 必须是合法的 FDT blob，且在程序生命周期内有效（'static）。
/// 解析失败或重复初始化都会 panic（板级描述损坏 / 重复 board_init 属致命错误）。
///
/// # Safety 约定
/// 单 hart 串行调用。若未来多 hart，需保证仅一个 hart 执行本函数且其它
/// hart 在 [`dt`] 返回就绪前不读取。
pub fn init_dtb(dtb: &'static [u8]) {
    // 期望由 board_init 在调度器启动前串行调用。
    if STATE
        .compare_exchange(
            STATE_UNINIT,
            STATE_INITING,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        panic!("init_dtb: already initialized");
    }

    let fdt = Fdt::from_bytes(dtb).expect("init_dtb: invalid FDT blob");

    // SAFETY: STATE == INITING 意味着我们是唯一持有者；单 hart 下无并发写入。
    // 转为 *mut Fdt 后用 ptr::write 写入，避免 Rust 2024 禁止的 static mut 引用。
    unsafe {
        let fdt_ptr: *mut Fdt<'static> = addr_of_mut!(FDT) as *mut Fdt<'static>;
        fdt_ptr.write(fdt);
    }

    STATE.store(STATE_READY, Ordering::Release);
}

/// 获取全局 FDT 句柄。必须在 [`init_dtb`] 之后调用。
///
/// # Panics
/// 若 [`init_dtb`] 尚未调用则 panic。
pub fn dt() -> &'static Fdt<'static> {
    if STATE.load(Ordering::Acquire) != STATE_READY {
        panic!("dt() called before init_dtb()");
    }
    // SAFETY: STATE == READY 保证 FDT 已被写入且对所有读者可见（Release/Acquire）。
    // FDT 是 static，其内部 Fdt 在 init_dtb 后不再被修改，故可安全返回 'static 引用。
    // 使用 addr_of_mut! 取裸指针避免 Rust 2024 禁止的 static mut 引用。
    // MaybeUninit<T> 与 T 内存布局一致，故 *mut MaybeUninit<T> 可安全转 *const T。
    // Fdt 自身持有 'static 切片引用（来源为 include_bytes! 的 'static 数据）。
    unsafe {
        let fdt_ptr: *const Fdt<'static> = addr_of_mut!(FDT) as *const Fdt<'static>;
        &*fdt_ptr
    }
}
