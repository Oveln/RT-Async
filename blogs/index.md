---
# https://vitepress.dev/reference/default-theme-home-page
layout: home

hero:
  name: "RT-Async"
  text: "基于 Rust 的 async RTOS 内核"
  tagline: 优先级抢占 + 共享系统栈 + 零额外上下文切换开销
  actions:
    - theme: brand
      text: 查看技术文档
      link: /docs/
    - theme: alt
      text: 阅读开发周报
      link: /周报-Oveln/

features:
  - title: 优先级抢占
    details: 高优先级 executor 抢占低优先级，通过 Pend ISR 在共享栈上嵌套运行
  - title: 共享系统栈
    details: 所有 executor 共用一个系统栈，不逐 task 分配栈空间，任务数量无上限
  - title: O(1) 调度
    details: 两级位图 trailing_zeros() 常数时间定位最高优先级就绪 executor
  - title: 零额外栈开销
    details: executor 切换复用 Rust 函数调用/返回语义，不经汇编上下文切换
  - title: #![no_std]
    details: 纯 Rust，无 libc 依赖，适合 RISC-V 裸机部署
  - title: 过程宏
    details: #[task]、#[main]、#[interrupt] 声明式 API，自动生成 ISR 和调度器接线
---
