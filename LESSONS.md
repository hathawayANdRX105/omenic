# omenic LESSONS（技术教训）

> 收录标准：**会重复犯的错**（规则可传递到下一次编码/审查）。一次性失误、已被工作约定覆盖的流程件不收录。
> 格式：规则 → 为什么（出处 PR）。交付史细节在 git 历史，本文只留可传递部分。

## 协议与序列化

1. **serde：enum 级 `rename_all` 不覆盖 struct-variant 字段**。变体含结构体字段时必须写**变体级** `#[serde(rename_all = "camelCase")]`，否则字段名以默认规则上线（`option_id` 事故）。且运行时 serde 错误本地 `cargo check` 抓不到——新协议层只能靠 CI 暴露。（#387）
2. **「字段存在但为空」≠「字段缺失」**。`as_str()` 命中空串会走进覆盖分支：网关续帧把 `id`/`name` 重复为 `""` 时，无条件覆盖把首帧真值冲掉，全部工具调用 `tool "" not found`。累积型解析器对覆盖必须加非空守卫。测试要用**真网关帧形**，不能只造理想帧——CI mock 遵循严格规范所以从不暴露。（#407）
3. **mock 必须字节级保真**。e2e mock 从只含 header 的 buffer 算 body 偏移 → 塌成 0，9401 字节请求只记录 8538，模型从未拿到完整工具表，循环静默 `agent_end`。mock 丢数据不报错，只会让被测系统「看起来正常」。（#385）

## 生产路径

4. **新能力上线前问一句「生产路径谁调用」**。G8 三例全是「单测绿、生产失效」：run 在 prompt ack 时就被关闭（事件流从不流经生产代码）、`save_to_file` 整文件重写抹掉 `[mcp]` 段、spill 文件名恒定互相覆盖。测试绿不等于接线绿。（#377/#378）
5. **RPC 客户端统一用 `call` 不用 `call_raw`**。`call_raw` 只解传输错误、不检查 `resp.success`——daemon 的业务错误全部变成 `Ok(())`，调用方降级分支成死代码，失败与成功不可区分。（#403）
6. **写路径的崩溃形态要想清楚**。append 分两次 `write_all`（记录字节、`\n`），崩溃落在两次之间是真实形态；trim 截断点必须取「剥尾换行后最后一个 `\n`+1」，否则每次崩溃恢复静默丢一条完整记录。（#400）

## 测试

7. **断言要能区分「删掉被测逻辑」**。vacuous guard 实例：`"\t> # 全部"` 的行首 tab 被 `body.trim()` 提前吃掉，断言无法区分闭包里删掉 `'\t'`——注释声称钉的行为根本没钉。写断言时自问：把被测的那行删掉，这个测试会红吗。（#405）

## 并发与生命周期

8. **持锁 notify；锁一律 poison 容忍**。drop guard 后再 notify，通知落在读者进 `wait_timeout` 的窗口内无人接收 → 空等满 timeout。`.lock().unwrap()` 全换 `lock_recover`（对齐 mcp/daemon 惯例）。（#385）
9. **进程级共享状态要串行化**。`Config::load`/`set_current_dir` 是进程级，cargo 并行线程下三用例互抢 → 加 `cwd_lock()`。（#385）
10. **dispose 路径防重入与赛跑**：worker 未 initialize 完就拆 stdin → broken pipe（三重门：共享 abort flag + 门拆后传输失败报 Aborted）；disposer 在 `wait_for_exit` 里重入自己的 Mutex guard → 全测试死锁（先算 `exited`、guard 出作用域再 take）。（#387）
11. **pty 杀进程看会话 id 不看进程组**：job control 给每个作业单独 pgrp，杀 shell pgrp 不够；portable-pty exec 前 `setsid`，真正共享的是会话 id。实测 SIGTERM 杀不掉 pty 上的 bash、SIGHUP 杀 shell 但 pty 永不 EOF，只有 SIGKILL 既 reap 又让 pty EOF。（#385）
12. **pty 回显不可靠，用哨兵**：`write("echo hi")` 后读端有两份 `hi`（回显+真输出），数出现次数会误判；让 shell 拼 `__DONE_<token>__` 哨兵字面量。（#385）

## 工程流程

13. **`git commit ... | tail -1` 会吞 hook 失败退出码**。之后 `--amend` 会把当期 HEAD（可能毫无关系的提交）当成修正对象——曾把 main tip 修正成「docs+fix」混合提交。commit 后必须显式 `echo $?`。（2026-09-21 实操）
14. **squash 合并后分支 tip 不是 main 祖先**，git 判「未合并」是正常形态；清理前用 `git branch --merged main` 区分真祖先合并与 squash 残留，`backup/*` 一律不动。
