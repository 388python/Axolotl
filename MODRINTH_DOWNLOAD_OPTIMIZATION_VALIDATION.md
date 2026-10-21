# Modrinth 下载优化验收记录

日期：2026-10-21。执行依据：`E:/MODRINTH_DOWNLOAD_OPTIMIZATION_PLAN.md`。

## 当前结论

实现与验收仍在进行。`AXOLOTL_MODRINTH_IP_SELECTION` 默认 `off`，不能将已通过的确定性恢复测试视为完整性能验收。尚未满足默认 `auto` 的启用门槛。

已实现 DoH 证据复用、有限 Fastly 候选规则、严格 TLS/Range 探测、共享协调器、真实 reqwest/H2 pinning、连接代次退休、同 URL 续传、连续低速 Range 恢复、扩容收益试验、缓冲输出、候选 hint 缓存及生产反馈回滚。代码和测试仍需按原计划逐项完成最终审计。

## 本轮审计修正

- 接收超时使用明确的流停滞错误，不增加 H2 协议故障评分。
- RST_STREAM 保留错误类型，只恢复对应流；连接级协议错误仍能触发回退。
- 正文截断、hash 或文件内容校验失败继续走完整性恢复，不直接推断整个 authority 的 H2 协议有问题。
- 实测更快的 HTTP/1 路径使用短期传输偏好，独立于协议失败 cooldown。
- 单一官方主机的自动测速由端点选择器负责；旧 URL 路由探测也受调节租约限制。
- 已有两个慢速窗口、10 秒最短观察时间在窗口中间满足时，不再无故多等一个完整窗口。生产首字节超时仍为 20 秒，二者是不同的计时器。
- 饱和主机只剩一个恢复槽时，候选验证串行推进，而非临时额度不足就跳过全部候选。
- 跨域跳转测试验证实际 peer 变化、普通客户端重选及 Authorization 移除。
- 显式网络上下文中的元数据请求复用该上下文，JSON 单元测试不再等待真实应用初始化。
- macOS 的系统代理模式在本优化中按路径未知处理；显式无代理和既有 Direct 路径可以参与。系统代理的原传输行为仍需跨平台验证。

## 确定性检查

以下结果分别对应其运行时的文件状态；后续修改须重跑受影响检查。

| 检查 | 结果 | 证据日志 |
|---|---|---|
| `cargo test -p theseus download -- --test-threads=1` | 293 passed，5 ignored，0 failed | `%TEMP%/axolotl-benchmark-flush-regression.log` |
| `cargo test -p theseus util::fetch::tests -- --test-threads=1` | 91 passed，0 failed | `%TEMP%/axolotl-fetch-complete-tests.log` |
| `cargo check -p theseus` | 通过 | `%TEMP%/axolotl-benchmark-stage-check.log` |
| `cargo fmt --all --check` | 通过 | 当前命令输出 |
| `git diff --check` | 通过 | 当前命令输出 |

Cargo 检查沿用配置的 target 和 profile，仅对命令进程临时设置 `CARGO_INCREMENTAL=0`，绕过此前已复现的 Rust 1.95 增量 fingerprint ICE。未清理缓存、切换 target、修改 Cargo 配置或启动桌面应用。

## 性能驱动

实现位于 `packages/app-lib/src/util/download/endpoint_selection/tests/benchmark.rs`，本地 TLS fixture 位于同目录的 `fixtures.rs`。显式 ignored 测试包含健康网络、慢入口/快备选、全部慢入口、慢尾部与 200 个混合大小文件的场景。

默认单文件规模为 64 MiB，`AXOLOTL_BENCH_SIZE_MIB` 可选择 1–128 MiB。非 64 MiB 的结果必须标为 scaled，不能声称完成 64 MiB 验收。每种单文件场景按三种模式、冷/暖连接各五轮交错执行，记录所有轮次及中位数/p95。

基准使用生产的首字节/读取超时，而非普通单元测试缩短后的超时。服务端显式 flush TLS 缓冲，使用应用层 pacing；这不是真实 TCP 丢包仿真。缓存状态仅模拟同一对象首次/后续访问，没有真实 CDN 回源成本。

`DnsOnly` 和 `Expanded` 在本地 fixture 中表示注入一组或扩展一组候选；生产 Fastly catalog 的前提与地址过滤由独立规则测试验证。这不能证明本地 IP 与真实 Fastly PoP 的对应关系。

## 健康网络测量：门槛未通过

命令：`cargo test -p theseus modrinth_download_benchmark_healthy -- --ignored --nocapture --test-threads=1`。

环境：本机 Windows 回环 HTTPS；64 MiB 确定性内容，目标 pacing 为 30 MiB/s。未访问公网，未使用用户代理出口进行速度排名。日志：`%TEMP%/axolotl-benchmark-healthy-flushed.log`。

| 连接 | 模式 | 轮数 | 中位完成时间 ms | p95 ms |
|---|---|---:|---:|---:|
| 冷 | off | 5 | 4310 | 4598 |
| 冷 | DNS 候选 | 5 | 4011 | 4121 |
| 冷 | 扩展候选 | 5 | 4345 | 4473 |
| 暖 | off | 5 | 2539 | 4592 |
| 暖 | DNS 候选 | 5 | 2683 | 4432 |
| 暖 | 扩展候选 | 5 | 4293 | 4555 |

所有 30 次下载均仅尝试一次，received/unique/server payload 均为每次 67,108,864 字节，probe bytes=0，IP migration=0，连接峰值=1。冷连接开启一条连接；暖连接复用一条现有连接。

冷连接 expanded 相对 off 的中位耗时增加约 0.81%，暖连接增加约 69.08%。因此测试按 5% 门槛以非零状态退出，不能记为通过。轮次存在明显波动，服务端 body_wall_time 也随之变化；目前证据不足以将其归因于调度、TLS、磁盘或选择器中的某一个因素，也不足以豁免回归。

先前两轮不采用为验收数据：第一轮继承测试超时并缺少 TLS flush，出现额外 64 KiB；第二轮改为生产超时但仍缺 flush，出现读超时重试，已停止本任务自己的该次测试进程。没有终止用户的开发或 Cargo 进程。

## 尚待完成的要求

- 定位并解决健康暖连接测量差异，或提供足以证明测量无效的独立证据，然后重新执行验收。
- 执行慢入口/快备选、全部慢入口、慢尾部及 200 文件对照矩阵；缩小规模不能替代完整规模的收益结论。
- 继续审核 H2 单流、H2 Range、HTTP/1 Range、内存下载、重定向和失败恢复的全部实际路径与计数覆盖。
- 补齐候选证据、计费网络、代理上下文、网络代次及过期结果的验收映射；记录未验证平台。
- 审核物理连接预算与库内部连接生命周期，包含竞速建连、旧代次、探测、恢复和取消。
- 检查恢复总尝试预算、探测干扰样本、正文预算和冷却是否在完整场景中按计划成立。
- 补齐稳定诊断字段与失败原因；不能用日志或策略 mock 替代真实连接证据。
- 完成真实网络可取得的验证。当前没有国内直连跨运营商提速数据，跨网络收益未验证。
- 最后按计划第 18 节逐项审计，并在满足门槛后处理默认 `auto`；此前保持 `off`。

本文件记录阶段证据和未完成项，不是整体完成声明。
