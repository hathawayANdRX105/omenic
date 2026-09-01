# Gate Checklist — 项目级自定义检查

`.githooks/spec/checklist_*.yaml` 是 omenic gate 的"项目级规则"入口。
每份 yaml = 一条检查，gate 把内容（git diff / 全文 / 静态扫描结果）喂给
任意 harness（可执行文件），harness 回传 finding JSON，gate 走既有
FAIL/WARN/INFO 出口。

不需要写 Rust，不需要碰 gate 源码，加 yaml 就完事。

## 何时用

| 场景 | 模式 | 例子 |
|---|---|---|
| 铁律类：禁路径 / 必放位置 / 文件名约定 | `mode: grep` | page 不许有 `src/ui.rs`；tests 必须 `test_` 前缀 |
| 增量评审：新代码是否符合规范 | `mode: diff` | diff 中不能有 `println!`；commit 信息不能"修复bug" |
| 全文判断：需要全局上下文 | `mode: file` | 文件 >800 行提示拆分；CJK 字面量重复 ≥2 次抽 const |

判断标准：**规则能不能用 grep 表达？** 能 → `mode: grep`（零 LLM，毫秒）。
不能（需要语义理解）→ LLM 模式（费 token，慢 1-3s）。

## 前置条件

```bash
# 1. 装 omenic gate 二进制 (CK-04 已实现)
#    拷 omenic PR #273 impl 编译出的 release 版
cp /path/to/omenic/.wt/271-feat-gate-checklist/target/release/gate \
   ~/.local/bin/gate

# 2. 装 LLM CLI (任选)
#    claude  → Anthropic CLI
#    codex   → OpenAI CLI
#    ollama  → 本地 ollama
#    9router → 9router 启动后
#    任意 OpenAI 兼容端点也 OK (用 llm-checklist-harness.sh)

# 3. jq (mode: grep 的 yaml 内嵌 sh 解析 finding JSON 时用)
sudo pacman -S jq
```

## 三步落地

### Step 1: 复制模板

```bash
cd /path/to/your-project
mkdir -p .githooks/spec

# 从 omenic 拷协议 + demo (CK-04 mode: grep 已合)
cp /path/to/omenic/.wt/271-feat-gate-checklist/.githooks/spec/CHECKLIST_SPEC.md \
   .githooks/spec/
cp /path/to/omenic/.wt/271-feat-gate-checklist/.githooks/spec/CHECKLIST_DEMO_README.md \
   .githooks/spec/
cp /path/to/omenic/.wt/271-feat-gate-checklist/.githooks/spec/llm-checklist-harness.sh \
   .githooks/spec/
chmod +x .githooks/spec/llm-checklist-harness.sh
```

### Step 2: 写 dispatch.yaml（gate 入口）

```bash
cat > .githooks/spec/dispatch.yaml <<'YAML'
pre-commit:
  - workspace
  - code
  - checklist

pre-push:
  - workspace
  - code
  - checklist

merge:
  - github/pull_requests
  - github/reviews
  - cleanup
  - checklist
YAML
```

注意：dispatch.yaml **必须**在 `.githooks/spec/`，**不是** `.githooks/`。
gate pre_commit/pre_push/merge 三个入口都从 spec/ 读。

### Step 3: gate init 接入 git hooks

```bash
gate init
# 自动做：
#   1. 部署 gate 到 ~/.local/bin/gate 和 ~/.local/bin/gh
#   2. 写 .githooks/hooks/{pre-commit, pre-push, merge} 三个 sh 包装
#   3. git config core.hooksPath = .githooks/hooks
# 已 init 过会跳过
```

如果 `gate init` 报 "could not find .githooks directory"，先 `mkdir -p .githooks/spec` 即可。

## 写一条规则 (yaml schema)

文件名必须是 `checklist_<name>.yaml`，按字典序跑（想控顺序加 `00_`/`10_`/`99_` 前缀）。

### 模式 1: grep（推荐，零 LLM）

```yaml
# .githooks/spec/checklist_no_print.yaml
# 禁 src/ 内 print() — 用 logging
enabled: true
hooks: [pre-commit, pre-push, merge]
fail_severity: FAIL        # 磨合期先 WARN,过了改 FAIL
match:
  paths_include: ["**/*.py"]
  paths_exclude: ["__pycache__/"]
mode: grep                  # ← 关键: harness 收空 stdin,自己跑 grep/find
harness:
  command: "sh"
  args:
    - "-c"
    # ROOT=$(git rev-parse --show-toplevel);
    # matches=$(grep -rFn 'print(' "$ROOT/src" --include='*.py');
    # 用 grep -F (固定字符串) 避免 ( 等元字符被当 regex
    # head -20 限制条数防爆 stdout
    - |
      ROOT=$(git rev-parse --show-toplevel);
      matches=$(grep -rFn 'print(' "$ROOT/src" --include='*.py' 2>/dev/null | head -20);
      if [ -z "$matches" ]; then
        echo '[]';
      else
        printf '%s' "$matches" | jq -R -s '
          split("\n")
          | map(select(length > 0))
          | map(capture("^(?<path>.+):(?<line>[0-9]+):(?<rest>.*)$"))
          | map({id: "NO-PRINT", severity: "WARN", path: .path, line: (.line|tonumber), message: ("用 logging: " + .rest)})
        ';
      fi
optional: true             # harness 缺失 → WARN 跳过(不强制装机)
timeout: 10
```

**关键陷阱**：
1. **`grep -F` 不要用 `-E`** — `print(` 的 `(` 是 regex 元字符，`-E` 解析失败
2. **`head -20` 防爆** — finding 数组无上限时大 diff 会卡
3. **`jq -R -s` 解析** — `-R` raw input, `-s` slurp 整个 stdin
4. **`capture("^(?<path>.+):(?<line>[0-9]+):(?<rest>.*)$")`** — 4 段：路径:行:内容，不要加 col 段（grep 没有）
5. **`-not -path '*/.wt/*'` 在 wt 内会自残** — 在 worktree 里跑时 `.wt/` 就是 cwd，所有路径都匹配 → find 永远空。直接去掉这个 filter

### 模式 2: diff（增量评审，需 LLM）

```yaml
# .githooks/spec/checklist_no_debug_log.yaml
# diff 中不能有 println!/dbg!
enabled: true
hooks: [pre-commit, pre-push, merge]
fail_severity: FAIL
match:
  paths_include: ["**/*.rs"]
mode: diff                  # ← stdin 收 git diff 全文
harness:
  command: "sh"
  args:
    - "-c"
    # cat | 调 LLM CLI. --print 必加, --output-format json 必加.
    # prompt 必须明写: "输出只允许 JSON 数组,没有命中输出 []"
    - "cat | claude --model haiku --print --output-format json '检查 git diff 中新增的 println!/dbg!/eprintln! 调用. 命中时输出 JSON 数组: [{id:DBG-01,severity:FAIL,path:\"<从diff提取>\",line:<新行号>,message:\"<调用> 不应留在生产代码\"}]. 没有命中输出 []. 不要其他输出.'"
optional: true
timeout: 30
```

**关键陷阱**：
1. **`cat | ` 必加** — gate 把 diff 灌 stdin，harness 必读 stdin
2. **prompt 必明写 `[]`** — 不然模型给 prose 解释，gate 解析失败
3. **`--print` / `--output-format json` 必加** — claude 默认带 prose
4. **pre-commit 慎用** — diff 检查快可以，file 检查慢放 pre-push/merge

### 模式 3: file（全文判断，需 LLM）

```yaml
# .githooks/spec/checklist_module_size.yaml
# 单文件 .rs > 800 行 → WARN
enabled: true
hooks: [pre-push, merge]   # 慢检查不放 pre-commit
fail_severity: WARN
match:
  paths_include: ["**/*.rs"]
  paths_exclude: ["target/", ".wt/", "**/tests/**"]
mode: file                  # ← 每个变更文件单独传,带 ===== FILE: <p> ===== 分隔符
harness:
  command: "sh"
  args:
    - "-c"
    - "cat | sh \"$(git rev-parse --show-toplevel)/.githooks/spec/llm-checklist-harness.sh\""
optional: true
timeout: 60
```

**关键陷阱**：
1. **必须有 `$(git rev-parse --show-toplevel)`** — 不能假设 cwd
2. **`mode: file` 把所有匹配文件拼起来灌 stdin** — 1 万行大文件会被 `head -c 102400` 截断
3. **每个文件前有 `===== FILE: <rel> =====`** — LLM 用这个把 finding path 写对

## 严重度合并

```
harness 报 FAIL + yaml fail_severity: WARN → 最终 FAIL
harness 报 WARN + yaml fail_severity: FAIL → 最终 FAIL
harness 报 INFO + yaml fail_severity: FAIL → 最终 FAIL
                                          （harness FAIL 永远阻断）
```

## 通用 OpenAI 兼容 harness

`llm-checklist-harness.sh` 支持任意 OpenAI 兼容端点（不是只能用 claude）。

```bash
# 默认读环境变量
export LLM_BASE_URL="https://apihub.agnes-ai.com/v1"  # 任意 OpenAI 兼容
export LLM_API_KEY="sk-xxx"
export LLM_MODEL="agnes-2.0-flash"   # 留空自动 /v1/models[0].id
export LLM_TIMEOUT=30
export LLM_PROMPT="额外 system 指令"

# Dry-run (不调 API,只 echo MOCK_FINDINGS) — 调试 yaml 用
LLM_DRY_RUN=1 MOCK_FINDINGS='[{"id":"X-01","severity":"WARN","line":1,"message":"test"}]' \
  gate pre-push
```

`llm-checklist-harness.sh` 完整源码在 omenic `.wt/271-feat-gate-checklist/.githooks/spec/`。
不在 omenic 仓库也没事 — 拷到任意项目 `.githooks/spec/` 就能用。

## Finding JSON 协议（harness 输出）

```json
[
  {
    "id": "DBG-01",                 // 必填,会拼成 <yaml文件名>.<id>
    "severity": "FAIL",             // 必填: FAIL | WARN | INFO
    "path": "src/foo.rs",           // 选填,相对仓库根
    "line": 42,                     // 选填
    "message": "println! left"      // 必填
  }
]
```

空数组 `[]` = 全 PASS。
stdout 不是 JSON → gate WARN "harness output not valid JSON"。

## 实际案例

### Case 1: ferrite 前端规范 (FR-01/02/04 用 grep, FR-03 用 LLM)

参考 `~/projects/ferrite/.wt/feat-checklist-grep/.githooks/spec/`：
- `checklist_structure_check.yaml` — 拦 `use mock::` 模式 (FR-01)
- `checklist_shared_components_check.yaml` — 拦 page `src/ui.rs` (FR-02)
- `checklist_tests_check.yaml` — 拦 `src/*_test.rs` (FR-04)
- `checklist_copy_constants_check.yaml` — LLM 判 CJK 重复 (FR-03)

ferrite PR: `feat/checklist-grep-frontend` (分支已推 origin)

### Case 2: sentinel (Python 项目, 3 条 grep)

参考 `~/projects/sentinel/.wt/feat-gate-demo/.githooks/spec/`：
- `checklist_no_print.yaml` — grep `print(` in src/
- `checklist_test_naming.yaml` — tests 函数必须 `test_` 前缀
- `checklist_review_yaml_sections.yaml` — review.yaml 必含 4 section

sentinel 本地分支 `feat/gate-checklist-demo` (无 remote)

### Case 3: API 路由必须 /v2/ 命名空间 (LLM diff)

```yaml
# .githooks/spec/checklist_api_v2.yaml
enabled: true
hooks: [pre-push, merge]
fail_severity: FAIL
match:
  paths_include: ["**/routes*.rs", "**/api*.rs"]
mode: diff
harness:
  command: "sh"
  args: ["-c", "cat | claude --model haiku --print --output-format json '检查 diff: 新增路由路径必须 /v2/... 开头. 若新增路径以 /v1/ 开头 → {id:API-01, severity:FAIL, line:<行号>, message:\"<path> uses v1, must be /v2/...\"}. 没有输出 [].'"]
optional: true
timeout: 30
```

### Case 4: 巨型 .rs 文件告警 (LLM file)

```yaml
# .githooks/spec/checklist_oversize.yaml
enabled: true
hooks: [pre-push, merge]
fail_severity: WARN
match:
  paths_include: ["**/*.rs"]
  paths_exclude: ["target/", ".wt/", "**/tests/**"]
mode: file
harness:
  command: "sh"
  args:
    - "-c"
    - "cat | sh \"$(git rev-parse --show-toplevel)/.githooks/spec/llm-checklist-harness.sh\""
optional: true
timeout: 60
```

`llm-checklist-harness.sh` 走 LLM_PROMPT 环境变量决定具体规则。可以在
`.githooks/spec/` 下放一份 prompts 目录，每个 yaml 用自己的 prompt。

## 调试技巧

```bash
# 1. 不调真 LLM,验 yaml 解析
LLM_DRY_RUN=1 MOCK_FINDINGS='[]' gate pre-push

# 2. 验 finding 路径输出
LLM_DRY_RUN=1 MOCK_FINDINGS='[{"id":"X","severity":"FAIL","path":"src/foo.rs","line":42,"message":"test"}]' \
  gate pre-push 2>&1 | grep X

# 3. 直接调 harness (不走 gate)
cat your_file.rs | LLM_DRY_RUN=1 MOCK_FINDINGS='[]' \
  sh .githooks/spec/llm-checklist-harness.sh

# 4. 跑全部 mock 用 CHECKLIST_DEMO_MOCK.sh
cp /path/to/omenic/.wt/271-feat-gate-checklist/.githooks/spec/CHECKLIST_DEMO_MOCK.sh \
   .githooks/spec/
chmod +x .githooks/spec/CHECKLIST_DEMO_MOCK.sh
# 临时把 harness.command 改成 sh .githooks/spec/CHECKLIST_DEMO_MOCK.sh
$EDITOR .githooks/spec/checklist_xxx.yaml
# 测
CHECKLIST_DEMO_FINDINGS='[{"id":"D","severity":"WARN","line":1,"message":"x"}]' \
  gate pre-push
```

## 临时禁用

```yaml
# 整条关
enabled: false

# 或只关某钩子
hooks: []

# 临时绕过 git hook
git push --no-verify
GIT_SKIP_HOOKS=1 git commit
```

## 故障排查

| 症状 | 原因 | 修 |
|---|---|---|
| gate 跑但 checklist 主题没出现 | dispatch.yaml 位置错 | 必须是 `.githooks/spec/dispatch.yaml`，不是 `.githooks/dispatch.yaml` |
| yaml 加载失败：`unknown field` | yaml 字段在 Rust struct 不存在 | 字段对齐: enabled/hooks/match/mode/harness/timeout/optional/fail_severity |
| yaml 加载失败：`missing field` | 必填字段没写 | harness.command 必填 |
| 跑通了但 finding 全空 | mode: file/diff 时 harness 没读 stdin | 检查 prompt 是否说"用 stdin 的内容" |
| 跑通了但 finding 全空 | mode: grep 时 harness 输出不是 JSON | `jq -R -s 'split("\n") | ...'` 链断了，单独跑测 |
| finding 出现但没阻断 commit | fail_severity 写 WARN | 改 FAIL；harness 报 FAIL 永远阻断 |
| `harness not installed` | 命令不在 PATH | 加 `optional: true`（默认）或不依赖 PATH 用绝对路径 |
| 跑通了但 mode: grep 没出 finding | `'/src/xxx.rs:'` 路径过滤写错 | 用 `/src/xxx.rs:`（不带前导 `:`） |
| 跑通了但 mode: grep 没出 finding | `print(` regex 解析失败 | 用 `grep -F` 不用 `-E` |
| 跑通了但 mode: grep 没出 finding | yaml 写在 wt 里但 `find` 用了 `'-not -path */.wt/*'` | 在 wt 内所有路径都匹配，删这个 filter |
| LLM 输出 prose 不是 JSON | prompt 没明说 "只输出 JSON 数组" | 加 "没有命中输出 []. 不要其他输出." |

## 跨项目复用

```bash
# 把整套拷过去
scp -r your-project/.githooks other-host:/path/to/other-project/

# 或 git submodule
git submodule add <repo-with-githooks> .githooks-std
# 然后 include (gate 不支持,但项目层可以 ln -s)
```

## 维护

- 加规则: 写新 `checklist_*.yaml` + commit
- 改规则: 改 yaml（不动 gate 二进制）
- 删规则: 删文件
- 改严重度: 改 `fail_severity: FAIL/WARN/INFO`
- 改触发钩子: 改 `hooks: [pre-commit, pre-push, merge]`

## 协议详细

`.githooks/spec/CHECKLIST_SPEC.md`（从 omenic 拷来）有完整协议：
- 三种 mode 的 stdin/argv 协议
- finding JSON 字段定义
- 严重度合并规则
- 退出码语义

CK-04 = `mode: grep` 是 ferrite 端需求倒推到 omenic 实现的（项目级规则
不只是 LLM，还可能是 grep/find 这种零成本静态检查）。

## Omenic 端参考

- impl PR: https://github.com/hathawayANdRX105/omenic/pull/273 (commits 累加)
- 协议 SPEC: omenic `.wt/271-feat-gate-checklist/.githooks/spec/CHECKLIST_SPEC.md`
- 测试: `cargo test -p spec --lib checklist` 13 个单元测试
- ferrite 端 PR: `feat/checklist-grep-frontend` 分支
- sentinel 端: `feat/gate-checklist-demo` 本地分支

## 写在最后

写规则最容易踩的坑（按频率排）：
1. yaml 字段不合法（忘了 `optional`/`timeout` 顶层化）
2. `mode: file` 时 harness 没读 stdin
3. `mode: grep` 时 `grep -E` 把 `(` 当元字符
4. `mode: grep` 时在 wt 内用 `-not -path '*/.wt/*'` 自残
5. `mode: grep` 时 `paths_include: ["crates/page-*/src/**/*.rs"]` 这种 glob
   当前 `matches_include` 不支持 → 用 `["**/*.rs"]` 配 `paths_exclude`
6. 路径过滤 `:/src/api.rs:` 漏了前导 `:` → 用 `/src/api.rs:`
7. 严重度合并是 max(harness, yaml)，harness 报 FAIL 永远阻断

新加规则前先看一遍"故障排查"那节，避开已知坑。
