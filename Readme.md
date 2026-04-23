# RT-Async

一个基于 Rust 的 async RTOS 内核，采用优先级抢占调度 + 同优先级协程协作的混合调度模型。

## 架构

系统以 **executor** 为调度单元，每个 executor 绑定一个优先级，仅有就绪任务的 executor 才会动态持有栈空间，任务按优先级分发到对应 executor 上执行：

- **跨优先级（抢占）：** 高优先级 executor 抢占低优先级。通过 O(1) 两级位图（`PriorityBitmap<N>`）在常数时间内定位最高优先级的就绪 executor。
- **同优先级（协作）：** 同一 executor 上的任务共享栈空间，通过 `.await` 让权，实现无需逐任务分配栈的轻量协作调度。

## 特性

- **任务数量无上限** — 栈仅由就绪的 executor 动态持有而非逐 task 分配，仅受并发 executor 数量限制
- **最多 4,096 个优先级** — 通过 `PriorityBitmap<N>` 配置（N × 64）
- **O(1) 就绪选择** — 两级 `u64` 位图，基于 `trailing_zeros()` 单条硬件指令完成查找，无查表开销
