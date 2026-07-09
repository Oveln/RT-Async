//! Device Tree Blob (DTB) 解析层。
//!
//! 板级初始化时调用 [`init_dtb`] 注入 DTB 切片（来源由板级决定：
//! 子模块内嵌 / 主仓库 esos 同款扫描 / 等），之后 [`dt`] 返回全局句柄供
//! driver probe 使用。
//!
//! `core::cell::OnceCell` 不是 `Sync`，无法直接用作裸机 `static`，故用
//! `MaybeUninit` 承载数据 + `AtomicBool` 标记是否已初始化。单 hart 串行
//! 调用（board_init 内一次），init 完成后只读。

use core::mem::MaybeUninit;
use core::ptr::addr_of_mut;

use fdt_parser::Fdt;
use portable_atomic::{AtomicBool, Ordering};

static INITED: AtomicBool = AtomicBool::new(false);
static mut FDT: MaybeUninit<Fdt<'static>> = MaybeUninit::uninit();

/// 注入 DTB 切片。由板级 `board_init` 调用，来源不限（内嵌/扫描/handoff）。
///
/// # Panics
/// `dtb` 必须是合法的 FDT blob，且在程序生命周期内有效（'static）。
/// 解析失败或重复初始化都会 panic（板级描述损坏 / 重复 board_init 属致命错误）。
pub fn init_dtb(dtb: &'static [u8]) {
    if INITED.swap(true, Ordering::SeqCst) {
        panic!("init_dtb: already initialized");
    }

    let fdt = Fdt::from_bytes(dtb).expect("init_dtb: invalid FDT blob");

    // SAFETY: INITED 刚置位，单 hart 下无并发写入。
    // 转为 *mut Fdt 后用 ptr::write 写入，避免 Rust 2024 禁止的 static mut 引用。
    unsafe {
        let fdt_ptr: *mut Fdt<'static> = addr_of_mut!(FDT) as *mut Fdt<'static>;
        fdt_ptr.write(fdt);
    }
}

/// 获取全局 FDT 句柄。必须在 [`init_dtb`] 之后调用。
///
/// # Panics
/// 若 [`init_dtb`] 尚未调用则 panic。
pub fn dt() -> &'static Fdt<'static> {
    if !INITED.load(Ordering::Acquire) {
        panic!("dt() called before init_dtb()");
    }
    // SAFETY: INITED 为真保证 FDT 已被写入。使用 addr_of_mut! 取裸指针避免
    // Rust 2024 禁止的 static mut 引用。Fdt 持有 'static 切片引用。
    unsafe {
        let fdt_ptr: *const Fdt<'static> = addr_of_mut!(FDT) as *const Fdt<'static>;
        &*fdt_ptr
    }
}
