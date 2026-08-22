#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""K3 IPC 延迟归因与优化——可视化（2026-08-16 → 08-23 板上实测数据）。

数据来源：user-test-bench dd/mb/s1/s2 场景 + rtbench + litmus，
全部为板上实测（闭环残差 0.0 校验）。单位如无说明均为 µs。
运行：python3 2026-08-23-K3-IPC延迟归因-make_figs.py   （在本目录下生成 5 张 PNG）
"""
import matplotlib
import matplotlib.pyplot as plt
import numpy as np

matplotlib.rcParams["font.family"] = "Noto Sans CJK SC"
matplotlib.rcParams["axes.unicode_minus"] = False

# 调色板：按「消耗类别」统一着色（全文档一致，方便讲解）
C_DATA = "#4CAF50"    # 数据搬运（裸访存）
C_FENCE = "#FF9800"   # 内存序 fence（Acquire/Release 载荷）
C_TIMER = "#E91E63"   # 计时器（mtime/mcycle 冷读开销）
C_BELL = "#2196F3"    # 门铃/中断/寄存器
C_COMP = "#9C27B0"    # 计算（postcard/dispatch）
C_AP = "#607D8B"      # AP 侧段
C_EST = "#B0BEC5"     # 估计值
C_TRUE = "#37474F"    # 真实执行

# ============================================================================
# 图 1：D1 路径 rtt=240µs 瀑布分解（dd 场景闭环恒等式，六段精确闭合）
# ============================================================================
def fig1_budget():
    # D1（睡眠唤醒路径）六段（dd n=30, P1 后, 闭环残差 0.0）
    segs = [
        ("AP 用户态发送 send",        8.5,  C_AP),
        ("ISR 舞步 ddrain",           3.6,  C_BELL),
        ("trap+调度+MSIP 落地 ddisp", 27.1, C_BELL),
        ("发现前缀 dpre",             24.3, C_TIMER),
        ("取包 try_recv drx",         45.6, C_FENCE),
        ("分发反序列化 dserde",       38.0, C_COMP),
        ("服务尾段+响应+门铃 S",      67.7, C_FENCE),
        ("AP 回程唤醒 APret",         25.3, C_AP),
    ]
    total = sum(v for _, v, _ in segs)
    assert abs(total - 240.0) < 0.2, total

    fig, ax = plt.subplots(figsize=(11, 4.2))
    left = 0
    for name, v, c in segs:
        ax.barh(0, v, left=left, color=c, edgecolor="white", height=0.55)
        ax.text(left + v / 2, 0.28, f"{name}\n{v:.1f}", ha="center", va="bottom",
                fontsize=8.5, rotation=0, linespacing=1.1)
        left += v
    ax.set_xlim(0, 245)
    ax.set_ylim(-0.75, 1.35)
    ax.set_yticks([])
    ax.set_xlabel("时间 (µs)")
    ax.set_title("图1  D1 路径单条消息往返 240µs 完整分解（dd 闭环恒等式，六段精确闭合）\n"
                 "dpre/drx/dserde 的分段值包含 mtime 时间戳读取开销，真实执行见图 3", fontsize=11)
    ax.axvline(total, color="k", lw=0.8)
    ax.text(total, -0.72, f"rtt = {total:.1f} µs", ha="right", fontsize=10, weight="bold")
    ax.spines[["top", "right", "left"]].set_visible(False)
    fig.tight_layout()
    fig.savefig("2026-08-23-k3-ipc-fig1-budget-waterfall.png", dpi=150)

# ============================================================================
# 图 2：每种操作的实测单价（log 轴，按类别着色）
# ============================================================================
def fig2_unit_price():
    ops = [
        # (名称, 单价 µs, 类别色)
        ("裸读 SRAM 同址（合并）",   0.022, C_DATA),
        ("裸读 SRAM 顺序跨行",        0.195, C_DATA),
        ("裸读 SRAM 冷跨行",          0.45, C_DATA),
        ("256B 槽整块读",              1.2,  C_DATA),
        ("256B 槽整块写",              6.3,  C_DATA),
        ("本地原子 RMW（CS 后端）",    0.09, C_FENCE),
        ("Acquire 原子读（ld+fence）", 2.2,  C_FENCE),
        ("Release 原子写（fence+sd）", 2.2,  C_FENCE),
        ("纯 fence",                   2.1,  C_FENCE),
        ("mtime MMIO 读（热循环）",    0.106, C_TIMER),
        ("mtime MMIO 读（间隔>15µs）", 24.0, C_TIMER),
        ("mcycle CSR 读（间隔态）",    3000.0, C_TIMER),
        ("counter1 读（间隔，已证伪）", 13.0, "#AED581"),  # AP 域跨互连 ~13µs 重锁开销，更换计时源已证伪
        ("AON_TIMER1 读（纯本地间隔）", 1.9, "#8BC34A"),  # 探针价；生产路径间隔仍 ~10µs 级（更换计时源第二次证伪）
        ("mailbox 寄存器读",           0.18, C_BELL),
        ("门铃 notify（fence+MMIO）",  3.5,  C_BELL),
        ("postcard 构+解 双向",        9.4,  C_COMP),
        ("dispatch 全程（含 handler）", 18.2, C_COMP),
    ]
    names = [o[0] for o in ops][::-1]
    vals = [o[1] for o in ops][::-1]
    cols = [o[2] for o in ops][::-1]
    fig, ax = plt.subplots(figsize=(10, 7))
    y = np.arange(len(ops))
    ax.barh(y, vals, color=cols, edgecolor="white")
    ax.set_yticks(y, names, fontsize=9)
    ax.set_xscale("log")
    ax.set_xlabel("单价 (µs, 对数刻度)")
    ax.set_title("图2  每种操作的实测单价——跨度 5 个数量级\n"
                 "同址读 22ns ↔ mcycle 冷读 3ms；主要开销：fence 2.2µs、mtime 间隔读 24µs", fontsize=11)
    for yi, v in zip(y, vals):
        ax.text(v * 1.15, yi, f"{v:g}", va="center", fontsize=8.5)
    ax.set_xlim(0.015, 9000)
    from matplotlib.patches import Patch
    ax.legend(handles=[Patch(color=C_DATA, label="数据搬运（裸访存）"),
                       Patch(color=C_FENCE, label="内存序 fence"),
                       Patch(color=C_TIMER, label="计时器读（mtime/mcycle）"),
                       Patch(color="#AED581", label="计时器读（counter1，已证伪）"),
                       Patch(color="#8BC34A", label="计时器读（AON_TIMER1，已证伪）"),
                       Patch(color=C_BELL, label="门铃/寄存器"),
                       Patch(color=C_COMP, label="计算")],
              loc="lower right", fontsize=9)
    ax.grid(axis="x", ls=":", alpha=0.5)
    fig.tight_layout()
    fig.savefig("2026-08-23-k3-ipc-fig2-op-unit-price.png", dpi=150)

# ============================================================================
# 图 3：RP 侧每消息分解——实测（含时间戳开销）vs 真实执行 vs 优化后
# ============================================================================
def fig3_rp_budget():
    # 左柱：实测分段（含 mtime 时间戳读取开销，dd 实测）
    meas = [("dpre 实测 24.3", 24.3, C_TIMER), ("drx 实测 45.6", 45.6, C_FENCE),
            ("dserde 实测 38.0", 38.0, C_COMP)]
    # 中柱：剥离 mtime 开销后的真实执行（每段减去段边界那次冷 mtime ≈24µs 的执行）
    real = [
        ("弹性前缀（set_busy+ch2 查）", 11.0, C_FENCE),
        ("取包（4 fence+槽读）",        19.8, C_FENCE),
        ("分发（match+postcard）",      11.0, C_COMP),
        ("响应发送（4 fence+槽写）",    15.1, C_FENCE),
        ("门铃 notify",                  3.4, C_BELL),
        ("ch2 收尾检查",                 6.6, C_FENCE),
        ("生产计时开销（mtime 1-2 次冷读）", 24.0, C_TIMER),
    ]
    # 右柱：P3+fence 清理后的目标（magic 缓存/自产索引 Relaxed/自旋瘦身）
    opt = [
        ("取包（2 fence+槽读）",        14.6, C_FENCE),
        ("分发",                        11.0, C_COMP),
        ("响应发送（2 fence+槽写）",    10.7, C_FENCE),
        ("门铃 notify",                  3.4, C_BELL),
        ("计时开销（采样制后）",           2.0,  C_TIMER),
    ]
    bars = [
        ("实测分段\n（含 mtime 时间戳开销）", meas),
        ("真实执行\n（剥离时间戳开销）", real),
        ("P3 优化后\n（目标）", opt),
    ]
    fig, ax = plt.subplots(figsize=(11, 6))
    for i, (title, segs) in enumerate(bars):
        bottom = 0
        for name, v, c in segs:
            ax.bar(i, v, bottom=bottom, color=c, edgecolor="white", width=0.55)
            if v >= 8:
                ax.text(i, bottom + v / 2, f"{name}\n{v:.1f}", ha="center", va="center",
                        fontsize=7.8, color="white", weight="bold")
            bottom += v
        ax.text(i, bottom + 2, f"Σ={bottom:.1f}", ha="center", fontsize=10, weight="bold")
    ax.set_xticks(range(len(bars)), [b[0] for b in bars], fontsize=10)
    ax.set_ylabel("µs")
    ax.set_title("图3  RP 侧处理一条消息的分解\n"
                 "左：dd 实测三段（每段多出约 24µs 的 mtime 冷读开销）\n"
                 "中：剥离后的真实执行 ≈91µs（含生产计时开销 24）\n"
                 "右：P3 优化后目标 ≈42µs", fontsize=11)
    ax.set_ylim(0, 130)
    ax.grid(axis="y", ls=":", alpha=0.5)
    ax.spines[["top", "right"]].set_visible(False)
    fig.tight_layout()
    fig.savefig("2026-08-23-k3-ipc-fig3-rp-message-budget.png", dpi=150)

# ============================================================================
# 图 4：mtime 间隔开销与 H8 扫描
# ============================================================================
def fig4_mtime_trap():
    fig, (ax1, ax2) = plt.subplots(1, 2, figsize=(11, 4.2))
    # 左：读计时器的单价 vs 间隔
    xs = [0.1, 7.8, 20, 20000]
    ys = [0.106, 1.0, 24.0, 24.0]
    ax1.plot([0.1, 7.8, 20, 50], [0.106, 1.0, 24, 24], "o-", color=C_TIMER, lw=2)
    ax1.set_xscale("log"); ax1.set_yscale("log")
    ax1.set_xlabel("距上一次读的间隔 (µs, log)")
    ax1.set_ylabel("单次读耗时 (µs, log)")
    ax1.set_title("mtime 读的间隔开销\n热 106ns → 间隔 20µs 后 24µs（231×）\n（mcycle 更慢：约 3ms/次，CSR 同样存在该问题）", fontsize=10)
    ax1.axvspan(15, 60, color=C_TIMER, alpha=0.08)
    ax1.text(16, 0.15, "深睡阈值\n~15µs", fontsize=8.5, color=C_TIMER)
    ax1.grid(ls=":", alpha=0.5)
    # 右：H8 新鲜写衰减（修正列错位后的有效数据）
    D = [0, 30, 300, 1000, 3000, 10000, 50000]
    t = [23.0, 14.2, 12.6, 11.7, 11.8, 11.8, 11.6]
    ax2.semilogx([max(d, 1) for d in D], t, "o-", color=C_DATA, lw=2, label="FRESH 单次 try_recv")
    ax2.axhline(11.7, ls="--", color="gray", lw=1)
    ax2.text(2000, 12.1, "基线 11.7（recv_seq 复刻价）", fontsize=8.5, color="gray")
    ax2.annotate("D=0 时高出 11.4µs\n大部分是时间戳引入的误差\n（实际新写入开销仅数 µs）",
                 xy=(1, 23.0), xytext=(150, 21), fontsize=8.5, color="#333",
                 arrowprops=dict(arrowstyle="->", color="#333"))
    ax2.set_xlabel("AP 写入 → RP 收取的间隔 D (µs, log)")
    ax2.set_ylabel("单次 try_recv (µs)")
    ax2.set_title("「新写入数据读取延迟」扫描（H8）\n排除了 posted 写未落地为主要原因的假设", fontsize=10)
    ax2.grid(ls=":", alpha=0.5)
    fig.suptitle("图4  两个关键现象的实测曲线", fontsize=11, y=1.02)
    fig.tight_layout()
    fig.savefig("2026-08-23-k3-ipc-fig4-mtime-trap.png", dpi=150, bbox_inches="tight")

# ============================================================================
# 图 5：优化路线收益叠加（D1/D2 两条轨迹）
# ============================================================================
def fig5_roadmap():
    fig, ax = plt.subplots(figsize=(11, 5))
    # ISR 直派已否决（08-21）；更换计时源已证伪（08-22，生产路径 mtime
    # 被交织流量保持活跃，无可省开销）——轨迹移除该步，P3 已验收（08-22
    # 板测 didx 7.6→3.1、同一次启动内 rtt −10µs）。
    steps = ["P1 基线\n(08-20)", "fence 去冗余\n(P3 已验收)", "双向轮询\n(W2)"]
    d1 = [240, 230, 219]
    d2 = [189, 179, 168]
    x = np.arange(len(steps))
    ax.plot(x, d1, "o-", lw=2.2, color=C_AP, label="D1 路径（睡眠唤醒）")
    ax.plot(x, d2, "s-", lw=2.2, color=C_FENCE, label="D2 路径（弹性自旋）")
    for xi, (a, b) in enumerate(zip(d1, d2)):
        ax.annotate(f"{a}", (xi, a), textcoords="offset points", xytext=(0, 8),
                    ha="center", fontsize=9, color=C_AP)
        ax.annotate(f"{b}", (xi, b), textcoords="offset points", xytext=(0, -14),
                    ha="center", fontsize=9, color=C_FENCE)
    ax.set_xticks(x, steps, fontsize=9.5)
    ax.set_ylabel("rtt (µs)")
    ax.set_title("图5  优化路线收益叠加（µs）\n"
                 "P3 已验收（08-22 板测：didx 7.6→3.1µs、同一次启动内 rtt −10µs、自旋 18→9.4µs/轮）；W2 已实测 −11µs（spin-await）\n"
                 "更换计时源已证伪（生产路径无可省开销）；ISR 直派已否决——处理留在 task（保持 rt-async 模型）", fontsize=10.5)
    ax.grid(ls=":", alpha=0.5)
    ax.legend(fontsize=10)
    ax.set_ylim(110, 260)
    ax.spines[["top", "right"]].set_visible(False)
    fig.tight_layout()
    fig.savefig("2026-08-23-k3-ipc-fig5-roadmap.png", dpi=150)

if __name__ == "__main__":
    for f in (fig1_budget, fig2_unit_price, fig3_rp_budget, fig4_mtime_trap, fig5_roadmap):
        f()
        print(f"{f.__name__} ✓")
    print("全部图已生成")
