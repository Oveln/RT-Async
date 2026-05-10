---
title: 调度器设计
date: 2026-05-09
type: description
---

# 调度器设计

RT-Async 的调度器由三层组件构成：`Spawner<N>` 管理全局调度，`Executor` 执行单优先级任务，`PriorityBitmap<G>` 提供 O(1) 优先级查找。

## Spawner

`Spawner<const N: usize>` 是全局调度器，`N` 为优先级数量（1..=4096）。

### 核心数据结构

```rust
pub struct Spawner<const N: usize> {
    executors: [Executor; N],                         // 每个优先级一个 Executor
    bitmap: Mutex<UnsafeCell<PriorityBitmap<64>>>,    // 优先级就绪位图
    prio_stack: Mutex<UnsafeCell<Vec<Priority, N>>>,  // LIFO 优先级栈
    pend: UnsafeCell<unsafe fn()>,                    // 平台 pend 回调
}
```

- **executors** — `N` 个 `Executor` 实例的数组，下标即优先级
- **bitmap** — 两级位图，记录哪些优先级有就绪任务（始终 64 组，避免 const generic 泛化 API）
- **prio_stack** — LIFO 栈，记录当前占据系统栈空间的 executor 优先级，栈顶为当前运行的 executor

### 关键方法

#### `init()`

将 bitmap 操作回调（`BitmapOps`）注入每个 Executor。必须在 pin 之后调用一次。

```rust
let mut spawner = Spawner::<4>::new();
let mut spawner = pin!(spawner);
spawner.as_mut().init();
```

#### `spawn(prio, token)`

将 `SpawnToken` 对应的任务派发到指定优先级的 Executor 运行队列。入队后检查是否需要触发 pend（有更高优先级就绪时）。

```rust
spawner.spawn(Priority::new(0), blink(13).unwrap());
```

#### `try_preempt() -> Option<RunToken>`

读取位图最高优先级，与 `prio_stack` 栈顶比较。若更高则 push 到栈顶并返回 `RunToken`，否则返回 `None`。

#### `run(run_token)`

执行 `RunToken` 对应优先级的 Executor，轮询该优先级运行队列上的所有就绪任务。

#### `complete_executor()`

从 `prio_stack` 弹出已完成 executor 的优先级，表示栈空间已回收。

## Executor

每个 `Executor` 绑定一个优先级，内部维护一个侵入式 FIFO 运行队列（`RunQueue`）。Executor 轮询队列中的每个 Future 直到队列为空。

Executor 通过 `BitmapOps` 回调与全局位图交互：
- 任务入队时 set 对应优先级位
- 任务出队后队列为空时 clear 对应优先级位

## PriorityBitmap

两级位图，支持 O(1) 查找最高优先级。

```
group_map (u64)       →  哪些组有就绪任务
group_table[0..G]     →  每组 64 个优先级的具体位
```

- `highest()` = `group_map.trailing_zeros()` 得到组号 → `group_table[g].trailing_zeros()` 得到位号 → `g * 64 + b`
- 两条 `trailing_zeros()` 指令，单周期完成，无查表

```
Capacity = NUM_GROUPS × 64
NUM_GROUPS ∈ [1, 64]  →  最多 4096 个优先级
```

### 操作

| 方法 | 复杂度 | 说明 |
|------|--------|------|
| `set(prio)` | O(1) | 设置优先级位 |
| `clear(prio)` | O(1) | 清除优先级位，组空时同步清除 group_map |
| `highest()` | O(1) | 返回最高就绪优先级 |
| `pop_highest()` | O(1) | 返回并清除最高优先级 |
| `is_set(prio)` | O(1) | 查询优先级是否就绪 |
| `is_empty()` | O(1) | 检查位图是否为空 |

## SpawnToken

`SpawnToken<F>` 是必须消费的令牌——drop 会 panic。这防止了任务被 spawn 后却从未调度的情况。唯一合法的消费路径是传给 `Spawner::spawn()`。
