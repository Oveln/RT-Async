---
title: 过程宏
date: 2026-05-09
type: description
---

# 过程宏

`executor-macro` crate 提供三个过程宏，简化任务定义和应用入口。

## #[task] — 任务定义

将 `async fn` 转换为可 spawn 的静态任务。

```rust
#[executor::task]
async fn blink(led: usize) {
    // ...
}

// 展开为：
// async fn __blink(led: usize) { ... }
// fn blink(led: usize) -> Result<SpawnToken<impl Future<Output = ()>>, SpawnError>
```

**展开行为：**
1. 原函数体重命名为 `__name`（内部 async fn）
2. 生成同名公开函数，返回 `SpawnToken`
3. 内部创建 `TaskStorage` 静态变量，调用 `.spawn()` 构造 token

**约束：**
- 必须是 `async fn`
- 返回类型必须是 `()`
- 不允许泛型参数和 `self` 参数

## #[main] — 应用入口

替代手写 `__rust_main` 和 `MachineSoft` ISR。宏自动完成平台初始化、调度器 ISR 接线和空闲循环。

```rust
#[executor::main]
fn main(spawner: Pin<&'static Spawner<4>>) {
    spawner.spawn(Priority::new(0), blink(13).unwrap());
    spawner.spawn(Priority::new(1), periodic().unwrap());
}
```

**生成内容：**

1. `static mut __SPAWNER` — `MaybeUninit<Spawner<N>>`，全生命周期静态
2. `__rust_main() -> !` — 裸机入口，依次执行 `platform::init()` → 创建 Spawner → 执行用户函数体 → `platform::start()` → WFI 空闲循环
3. `MachineSoft(&mut TrapFrame)` — RISC-V MSI 中断处理，根据 `PEND_MARKER` 区分调度器 pend 和外部 MSI

**约束：**
- 不能是 `async fn`
- 有且仅有一个 `Pin<&Spawner<N>>` 参数（宏从 const generic 提取 N）
- 不允许泛型参数

## #[interrupt] — 中断处理

将函数标记为 RISC-V 中断处理函数。

```rust
#[executor::interrupt]
fn MachineTimer(_tf: &mut TrapFrame) {
    // 清除定时器中断、更新 tick 等
}
```

**符号映射：**

| 函数名 | 输出符号 | 说明 |
|--------|---------|------|
| `MachineTimer` | `MachineTimer` | 直接映射 |
| `MachineExternal` | `MachineExternal` | 直接映射 |
| `MachineSoft` | `__Inner_MachineSoft` | 特殊：避免与 `#[main]` 冲突 |
| 其他 | 同函数名 | 直接映射 |

### MachineSoft 特殊处理

`MachineSoft` 符号被 `#[main]` 宏占用于调度 ISR。用户用 `#[interrupt]` 定义 `MachineSoft` 时，宏自动重写为 `__Inner_MachineSoft`。运行时当 MSI 非调度器触发时，生成的 `MachineSoft` ISR 会调用此函数。若未定义，链接器提供弱符号 `DefaultHandler`（abort）。

**约束：**
- 不能是 `async fn`
- 必须有且仅有一个 `&mut TrapFrame` 参数
