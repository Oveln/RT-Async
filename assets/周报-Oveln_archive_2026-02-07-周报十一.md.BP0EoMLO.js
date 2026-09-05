import{_ as a,c as n,o as p,a0 as e}from"./chunks/framework.C1J0Vcxi.js";const d=JSON.parse('{"title":"第十一周：修改embassy_preempt的内存位置，调试内存管理部分的代码","description":"","frontmatter":{"title":"第十一周：修改embassy_preempt的内存位置，调试内存管理部分的代码","date":"2026-02-07"},"headers":[],"relativePath":"周报-Oveln/archive/2026-02-07-周报十一.md","filePath":"周报-Oveln/archive/2026-02-07-周报十一.md"}'),t={name:"周报-Oveln/archive/2026-02-07-周报十一.md"};function l(i,s,r,c,o,_){return p(),n("div",null,[...s[0]||(s[0]=[e(`<p>在调试中发现embassy_preempt的堆栈区域和opensbi或uboot踩踏，hart1报错（hart1上运行的是opensbi）</p><div class="language- vp-adaptive-theme"><button title="Copy Code" class="copy"></button><span class="lang"></span><pre class="shiki shiki-themes github-light github-dark vp-code" tabindex="0"><code><span class="line"><span>U-Boot SPL 2026.01-rc4-00010-ge55a81c36843-dirty (Jan 01 1980 - 00:00:00 +0000)</span></span>
<span class="line"><span>DDR version: dc2e84f0.</span></span>
<span class="line"><span>Trying to boot from SPI</span></span>
<span class="line"><span>UART logger initialized</span></span>
<span class="line"><span>==H=a OrSt In0i tsk Siptparintegd  c=ol=d=b</span></span>
<span class="line"><span>oOoStI</span></span>
<span class="line"><span>leHta:rt  OS4I nsiktiHpopoiknBge gcionl dcboomotp</span></span>
<span class="line"><span>  tHaedrt</span></span>
<span class="line"><span> O2S sInkiitp:pi nAbgo ucto ltdob oocatl</span></span>
<span class="line"><span>pH Iarnitt _3H eskaip</span></span>
<span class="line"><span> pOinSIgn ciotl:d Ibnooitt_</span></span>
<span class="line"><span>ngHaapr t co0m: plJeumtepid</span></span>
<span class="line"><span>InO StIon iptay:l oAbaodu tat t 0o xc8a0l4l00 O00S_0</span></span>
<span class="line"><span>  itStackAllocator</span></span>
<span class="line"><span>OSInit: OS_InitStackAllocator completed</span></span>
<span class="line"><span>OSInit: About to call init_platform</span></span>
<span class="line"><span>OSInit: init_platform completed</span></span>
<span class="line"><span>bSInit: A</span></span>
<span class="line"><span> sobuti _ttora cpa_lelr roGlr:ob halaSrytn1:cE xtercaup0to:r i</span></span>
<span class="line"><span>roleSIganil t:in GsltorbuactlSioynnc Ehaxendcluteror  fcaiomlpelde t(eedr</span></span>
<span class="line"><span>  Or SI-2n)it</span></span>
<span class="line"><span> A</span></span>
<span class="line"><span>: sbobiu_tt rtoap c_ealrrl orOS:_ Ihanirtt1Ta:s tkIrdaple0</span></span>
<span class="line"><span>0 OmScaInusite=: 0OxS0_00In0i0t0T0a00sk00I0dl00e 02c ommptlvaetl=ed0x</span></span>
<span class="line"><span>o 0S0I00n0it0:0 0A00bo0u00t0 t0</span></span>
<span class="line"><span>: csbalil_ tOrSap_I_enritroErve: nthaLirts1t</span></span>
<span class="line"><span>t tOSrIanpi0:t:  mOeSp_c=In0xit00Ev00en00tL00is4t00 c04o0mp0l4e tmsedta</span></span>
<span class="line"><span>=u=s=== 0xOS0I0n00it00 C0ao0mp0l0e01te8d00 =</span></span>
<span class="line"><span>bi=</span></span>
<span class="line"><span>  H_terlalop,_e Errmboras: syha Prrte1:em pttr aop0n :V irsai=0onx0F0iv00e02!0</span></span>
<span class="line"><span>40003d2e sp=0x000000004004ae30                                                00</span></span>
<span class="line"><span>sbi_trap_error: hart1: trap0: gp=0x0000000000000000 tp=0x000000004004b000</span></span>
<span class="line"><span>sbi_trap_error: hart1: trap0: s0=0x000000004004aeb0 s1=0x000000004004b0e0</span></span>
<span class="line"><span>sbi_trap_error: hart1: trap0: a0=0x00000000400003c8 a1=0x00000003fffffc00</span></span>
<span class="line"><span>sbi_trap_error: hart1: trap0: a2=0x000000004004ae30 a3=0x0000000000000000</span></span>
<span class="line"><span>sbi_trap_error: hart1: trap0: a4=0x000000004004ae30 a5=0x0000000040040238</span></span>
<span class="line"><span>sbi_trap_error: hart1: trap0: a6=0x0000000100010c00 a7=0x000000004004ae58</span></span>
<span class="line"><span>sbi_trap_error: hart1: trap0: s2=0x000000004004b000 s3=0x0000000040043220</span></span>
<span class="line"><span>sbi_trap_error: hart1: trap0: s4=0x0000000040043230 s5=0x0000000040043238</span></span>
<span class="line"><span>sbi_trap_error: hart1: trap0: s6=0x0000000000000001 s7=0x0000000000000005</span></span>
<span class="line"><span>sbi_trap_error: hart1: trap0: s8=0x0000000000002000 s9=0x0000000040043924</span></span>
<span class="line"><span>sbi_trap_error: hart1: trap0: s10=0x0000000000000000 s11=0x0000000000000000</span></span>
<span class="line"><span>sbi_trap_error: hart1: trap0: t0=0x0000000000002000 t1=0x0000000000000000</span></span>
<span class="line"><span>sbi_trap_error: hart1: trap0: t2=0x0000000000001000 t3=0x0000000000000000</span></span>
<span class="line"><span>sbi_trap_error: hart1: trap0: t4=0x0000000000000000 t5=0x0000000000000000</span></span>
<span class="line"><span>sbi_trap_error: hart1: trap0: t6=0x0000000000000000</span></span></code></pre></div><p>发现之前将embassy_preempt放在0x80400000实际上是非法地址，星光2的启动阶段内存安排是这样的</p><p><img src="https://doc.rvspace.org/VisionFive2/Developing_and_Porting_Guide/JH7110_Boot_UG/Image/JH7110_SDK/Boot_Process.svg" alt="alt text"></p><p>为了保证之后embassy_preempt和linux之间不互相踩踏内存，需要在各个系统间设定一块embassy_preempt专用的内存区域</p><p>先将原先的0x80400000改为0x40800000，产生spl阶段错误</p><div class="language- vp-adaptive-theme"><button title="Copy Code" class="copy"></button><span class="lang"></span><pre class="shiki shiki-themes github-light github-dark vp-code" tabindex="0"><code><span class="line"><span>SPL: Unsupported Boot Device!</span></span>
<span class="line"><span>SPL: failed to boot from all boot devices</span></span></code></pre></div><p>改为0x81000000，opensbi能成功进入uboot，但embassy_preempt跳转后hart0崩溃</p><div class="language- vp-adaptive-theme"><button title="Copy Code" class="copy"></button><span class="lang"></span><pre class="shiki shiki-themes github-light github-dark vp-code" tabindex="0"><code><span class="line"><span>U-Boot SPL 2026.01-rc4-00010-ge55a81c36843-dirty (Jan 01 1980 - 00:00:00 +0000)</span></span>
<span class="line"><span>DDR version: dc2e84f0.</span></span>
<span class="line"><span>Trying to boot from SPI</span></span>
<span class="line"><span>rt0: trap0: t0=0x0000040000000000 t1=0x0000000000000000</span></span>
<span class="line"><span>sbi_trap_error: hart0: trap0: t2=0x0000000000000000 t3=0x0000000000000000</span></span>
<span class="line"><span>sbi_trap_error: hart0: trap0: t4=0x0000000000000000 t5=0x0000000000000000</span></span>
<span class="line"><span>sbi_trap_error: hart0: trap0: t6=0x0000000000000000</span></span>
<span class="line"><span></span></span>
<span class="line"><span>OpenSBI v1.7-90-g8442b8df</span></span>
<span class="line"><span>   ____                    _____ ____ _____</span></span>
<span class="line"><span>  / __ \\                  / ____|  _ \\_   _|</span></span>
<span class="line"><span> | |  | |_ __   ___ _ __ | (___ | |_) || |</span></span>
<span class="line"><span> | |  | | &#39;_ \\ / _ \\ &#39;_ \\ \\___ \\|  _ &lt; | |</span></span>
<span class="line"><span> | |__| | |_) |  __/ | | |____) | |_) || |_</span></span>
<span class="line"><span>  \\____/| .__/ \\___|_| |_|_____/|____/_____|</span></span>
<span class="line"><span>        | |</span></span>
<span class="line"><span>        |_|</span></span>
<span class="line"><span></span></span>
<span class="line"><span>Platform Name               : StarFive VisionFive 2 v1.3B</span></span>
<span class="line"><span>Platform Features           : medeleg</span></span>
<span class="line"><span>Platform HART Count         : 5</span></span>
<span class="line"><span>Platform IPI Device         : aclint-mswi</span></span>
<span class="line"><span>Platform Timer Device       : aclint-mtimer @ 4000000Hz</span></span>
<span class="line"><span>Platform Console Device     : uart8250</span></span>
<span class="line"><span>Platform HSM Device         : ---</span></span>
<span class="line"><span>Platform PMU Device         : ---</span></span>
<span class="line"><span>Platform Reboot Device      : pm-reset</span></span>
<span class="line"><span>Platform Shutdown Device    : pm-reset</span></span>
<span class="line"><span>Platform Suspend Device     : ---</span></span>
<span class="line"><span>Platform CPPC Device        : ---</span></span>
<span class="line"><span>Firmware Base               : 0x40000000</span></span>
<span class="line"><span>Firmware Size               : 365 KB</span></span>
<span class="line"><span>Firmware RW Offset          : 0x40000</span></span>
<span class="line"><span>Firmware RW Size            : 109 KB</span></span>
<span class="line"><span>Firmware Heap Offset        : 0x4e000</span></span>
<span class="line"><span>Firmware Heap Size          : 53 KB (total), 0 KB (reserved), 13 KB (used), 39 KB (free)</span></span>
<span class="line"><span>Firmware Scratch Size       : 4096 B (total), 416 B (used), 3680 B (free)</span></span>
<span class="line"><span>Runtime SBI Version         : 3.0</span></span>
<span class="line"><span>Standard SBI Extensions     : base</span></span>
<span class="line"><span>Experimental SBI Extensions : none</span></span></code></pre></div><p>猜测是仍然内存被踩踏，只是由原先embassy_preempt破坏opensbi的内存区域变成opensbi破坏embassy_preempt的内存区域</p>`,10)])])}const b=a(t,[["render",l]]);export{d as __pageData,b as default};
