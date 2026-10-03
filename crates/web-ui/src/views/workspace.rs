//! 工作区页：dsh AppFrame 三列布局（侧栏 280 / 中栏）。
//!
//! 唯一数据源是本机 daemon（G5 删 mock）：启动时探测 + ping，连不上就是
//! 空态——空项目列表、空会话列表、空状态行、空任务看板，既不回退假数据
//! 也不 panic。assistant 流走 daemon `event.subscribe`（`worker_event_loop`
//! 读线程 → channel → 消费 task）。

use std::collections::{HashMap, HashSet};

use crate::components::chat::Chat;
use crate::components::dual_sidebar::DualSidebar;
use crate::components::taskpanel::TaskPanel;
use crate::components::ui::Modal;
use crate::layouts::app_frame::AppFrame;
use crate::nav::{SettingsSection, View};
use crate::shared as sh;
use crate::state::actions::{
    WorkspaceSignals, abort_run, add_space, answer_question, archive_session, change_model,
    create_session, delete_space, select_space, send_message, set_thinking,
};
use crate::state::backend::{DataBackend, daemon_space, probe_backend};
use crate::state::readiness::ReadinessGate;
use crate::state::session::{
    active_session_running, apply_run_statuses, merge_run_statuses, now_ms, session_exists_in,
};
use crate::state::subscriptions::{question_event_loop, worker_event_loop};
use crate::views::archive::ArchiveView;
use crate::views::config::SettingsPage;
use crate::views::stats::StatsView;
use dioxus::prelude::*;
use ui_kit::icons::{ANIM_SCOPE, IconChevronLeft, IconChevronRight};
use web_client::QuestionAnswer;
use web_client::QuestionItem;
use web_client::llm::LlmRuntimeConfig;
use web_state::convert::infer_session_status;
use web_state::types::{
    ChatMessage, PendingAttachment, Session, SessionStatus, StatusLine, TaskItem, WorkspaceSpace,
};
use web_state::ui_state::{AgentEvent, UiState};
use web_state::{is_placeholder_title, title_from_first_message};

const RUN_TASK_LIMIT: u32 = 50;
/// 双栏侧栏折叠态：全局信号，切视图后仍保持（宽度固定为 kit 双栏几何
/// （展开 270 / 折叠 80，见 app_frame grid_cols），拖拽调宽已退役）。
static SIDEBAR_COLLAPSED: GlobalSignal<bool> = GlobalSignal::new(|| false);

/// 任务看板拉取的任务存储条数上限（daemon `task.list` 的缺省值一致：
/// 按 `updated_at` 降序取最近若干条，跨会话的持久编排全在此列）。
const TASK_BOARD_LIMIT: u32 = 50;

#[component]
pub fn Workspace(
    config: LlmRuntimeConfig,
    on_update_config: EventHandler<LlmRuntimeConfig>,
) -> Element {
    // ── 数据后端：探测 daemon，连不上就是空态 ─────────────────────────────
    // 连接 + ping 放独立线程执行（UDS 往返通常 <10ms），主渲染线程最多等
    // 2s（`recv_timeout`）。daemon 卡住（socket 存在但 peer 不响应）时首帧
    // 不再冻死：超时退化为 Disconnected，空态先渲染，数据由事件订阅补。
    // 保留 JoinError 日志——socket 解析或 ping 里的真 panic 不能静默吞。
    let mut backend = use_signal(probe_backend);
    // 归档页刷新触发器：本页 restore/purge 后自增，ArchiveView 重取列表。
    let archive_rev = use_signal(|| 0u32);

    // Worker subscription readiness gate (shared between worker_event_loop
    // and the background prompt thread).
    let readiness = use_signal(|| ReadinessGate::new());

    let data_dir = config.data_dir.clone();
    let mut spaces = use_signal(move || match backend() {
        // Daemon 模式：项目 = 注册表（A2），首帧线程内取一次（LiveView 不能同步 RPC）；
        // 取不到（无 daemon / 超时）退化为单行 data_dir 项目（旧行为），不回退假数据。
        DataBackend::Daemon(d) => std::thread::spawn(move || d.list_projects())
            .join()
            .ok()
            .and_then(|r| r.ok())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| vec![daemon_space(&data_dir)]),
        // 无 daemon：无项目行（空态）
        DataBackend::Disconnected => Vec::new(),
    });
    let mut active_space_path = use_signal(|| {
        spaces
            .read()
            .iter()
            .find(|s| s.is_active)
            .map(|s| s.path.clone())
            .unwrap_or_default()
    });
    let data_dir = config.data_dir.clone();
    let mut space_sessions: Signal<HashMap<String, Vec<Session>>> = use_signal(move || {
        match backend() {
            // Daemon 模式：sidebar 会话列表 = list_sessions(50)，挂到唯一真实项目下
            DataBackend::Daemon(d) => {
                let path = daemon_space(&data_dir).path;
                let list = std::thread::spawn(move || d.list_sessions(50).unwrap_or_default())
                    .join()
                    .unwrap_or_default();
                HashMap::from([(path, list)])
            }
            // 无 daemon：没有任何会话（空态），不造假数据
            DataBackend::Disconnected => HashMap::new(),
        }
    });
    let mut active_session_id = use_signal(|| {
        space_sessions
            .read()
            .get(&active_space_path())
            .and_then(|list| list.first().map(|s| s.id.clone()))
            .unwrap_or_default()
    });
    let mut session_messages: Signal<HashMap<String, Vec<ChatMessage>>> = use_signal(HashMap::new);
    let pending_titles: Signal<HashMap<String, String>> = use_signal(HashMap::new);
    // 选中会话 → 填充消息：Daemon 线程内 load_messages(100)（线程 + join，
    // 仿旧 db_load_sessions 模式）；已缓存的会话不重复拉取。无 daemon 时
    // 不可能有选中会话（会话列表本身是空的），空态直接返回
    use_effect(move || {
        let sid = active_session_id();
        if sid.is_empty() || session_messages.read().contains_key(&sid) {
            return;
        }
        match backend() {
            // 无 daemon：没有消息来源，空态（不写 entry，留给发送路径按需建）
            DataBackend::Disconnected => {}
            DataBackend::Daemon(d) => {
                let sid_loaded = sid.clone();
                let msgs = std::thread::spawn(move || {
                    d.load_messages(&sid_loaded, 100).unwrap_or_default()
                })
                .join()
                .unwrap_or_default();
                session_messages.write().insert(sid, msgs);
            }
        }
    });

    // ⌘K 快速切换的 Daemon 搜索结果信号（Daemon 模式由下方 effect 维护；
    // 无 daemon 时恒空）
    let mut switcher_sessions: Signal<Vec<Session>> = use_signal(Vec::new);

    // 状态行：零值起步（G5 去 mock），只有真实来源的字段才填——model 来自
    // 运行配置，cwd 取 web 进程工作目录；git_branch 暂无数据源留空。
    // token/cost/耗时由 TurnEnd 结算与 start_run/finish_run 写入。
    let mut statusline = use_signal(|| {
        let mut st = StatusLine::empty();
        st.model = config.model.clone();
        // T4：active 模型声明的 context window（providers.toml）；未声明（0）
        // 回落引擎默认预算。
        st.context_max = if config.context_max > 0 {
            config.context_max as u64
        } else {
            web_state::types::DEFAULT_CONTEXT_MAX
        };
        st.cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        st
    });
    let mut view = use_signal(|| View::Chat);
    let mut show_quick_switcher = use_signal(|| false);
    // A2 创建项目目录弹窗（开合 + 待提交的目录路径草稿）。
    let mut show_create_project = use_signal(|| false);
    let mut create_project_path = use_signal(String::new);
    // 统计/设置二级菜单选项写入的页内定位信号（侧栏菜单与页面胶囊单一事实源）
    let stats_range = use_signal(|| String::from("24h"));
    let settings_section = use_signal(|| SettingsSection::Models);
    let mut show_tasks = use_signal(|| false);
    let mut search_query = use_signal(String::new);

    // ⌘K 打开/输入时：Daemon 模式优先 search_sessions（空 query 列 daemon
    // 全量）。异步化 + 代际 token：渲染线程不被阻塞 RPC 拖住，慢返回的旧
    // 查询不会覆盖新输入的结果（gen 不匹配即丢弃）
    let mut switcher_generation = use_signal(|| 0u64);
    use_effect(move || {
        if !show_quick_switcher() {
            return;
        }
        let DataBackend::Daemon(d) = backend() else {
            return;
        };
        let q = search_query();
        let gen_id = switcher_generation() + 1;
        switcher_generation.set(gen_id);
        spawn(async move {
            let list = d.search_sessions(&q, 50).unwrap_or_default();
            if switcher_generation() == gen_id {
                switcher_sessions.set(list);
            }
        });
    });

    // 订阅管线：读线程（worker_event_loop）→ unbounded channel → 消费
    // task。消费端与组件作用域绑定：VirtualDom 卸载 → task 取消 → rx
    // drop → 读线程 send 失败自行退出。订阅事件不带会话归属，统一写给
    // on_send 时记录的 run_target_sid；effect 只依赖 backend（初始化后
    // 不再变化），管线整个生命周期只建一次。
    let run_target_sid = use_signal(String::new);
    // WP-C：当前在飞 run 的 id（on_send 生成，随 worker.prompt 一起带给
    // daemon 记进 run ledger）。列表状态推断用它区分「正在跑」与「半开孤儿」；
    // TurnEnd / 中断时清空。空串 = 无在飞 run。
    let mut live_run_id = use_signal(String::new);
    // WP-C：看板数据版本号。todo/goal 的唯一写入口是模型工具（看板无编辑
    // RPC），刷新靠事件驱动：ToolResult 命中五个 todo/goal 工具名、以及
    // TurnEnd 收尾各 bump 一次。双触发是因为 WireTranslator 的 LIFO 配对
    // 会丢未配对 end（见 `convert.rs` 的 inflight 处理），只挂 ToolResult
    // 会漏刷新。WP-C 看板 effect 的依赖表读它即重查。
    let mut board_version = use_signal(|| 0u64);
    // WP-C：会话列表状态推断缓存——daemon 的 SessionSummary 无状态字段，
    // 列表侧的 Idle/Active/Aborted 改由 run.list 的 run 记录组装。为避免
    // 每次渲染都重复请求，run 记录只在会话列表数据变化（加载/新建/删除）
    // 时重查一次；无 daemon 时不接 run.list，缓存恒空，渲染侧归一为原状态。
    let mut run_status_cache: Signal<HashMap<String, SessionStatus>> = use_signal(HashMap::new);
    // 用户问题卡（plan-mode review）：订阅 user.question 推送 + 启动快照
    // 兜底（订阅建立前已提交的问题推送不补发，只有快照看得到）。与 worker
    // 事件管线同构：读线程只拥有克隆与 channel，Signal 全在消费侧碰。
    let mut pending_question = use_signal(|| None::<QuestionItem>);
    // 订阅管线一次性守卫（known-issue 1）：Disconnected 挂载时管线 effect
    // 直接早退；重探成功切到 Daemon 后 backend 变化重跑 effect，守卫防止
    // 多份读线程并存的重复建。
    let mut q_pipeline_started = use_signal(|| false);
    let mut w_pipeline_started = use_signal(|| false);
    use_effect(move || {
        if (q_pipeline_started)() {
            return;
        }
        let DataBackend::Daemon(d) = backend() else {
            return;
        };
        let (q_tx, mut q_rx) = tokio::sync::mpsc::unbounded_channel::<QuestionItem>();
        // 快照兜底：订阅建立前已提交的问题推送不补发，只有这条路径看得到。
        // 只挂第一个（plan review 一次一个；多个排队时回答掉当前的，剩下
        // 的等下一帧推送或下次快照）
        let d_snapshot = d.clone();
        let q_snapshot_tx = q_tx.clone();
        std::thread::spawn(move || {
            if let Ok(list) = d_snapshot.pending_questions() {
                if let Some(first) = list.into_iter().next() {
                    let _ = q_snapshot_tx.send(first);
                }
            }
        });
        std::thread::spawn(move || question_event_loop(d, q_tx));
        spawn(async move {
            while let Some(item) = q_rx.recv().await {
                pending_question.set(Some(item));
            }
        });
        q_pipeline_started.set(true);
    });
    use_effect(move || {
        if (w_pipeline_started)() {
            return;
        }
        let DataBackend::Daemon(d) = backend() else {
            return;
        };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
        let d_consumer = d.clone();
        let ready = readiness();
        let ready_for_worker = ready.clone();
        std::thread::spawn(move || worker_event_loop(d, tx, ready_for_worker));
        spawn(async move {
            let mut total_out: u64 = 0;
            // 高频 delta（AssistantText/Reasoning/ToolCall/ToolStart）按 80ms
            // 窗口收集，窗尾一次性写回会话、只触发一次信号（LiveView 重渲
            // 上限 ~12 次/秒——aui 平滑对位：思考块不再逐 delta 重解析闪
            // 跳）。结构性事件（TurnStart/TurnEnd/ToolResult）仍按原顺序逐条
            // 处理：先把其前挂起的高频 delta 落库，再处理自身——它的读写看
            // 到完整状态（窗内 TurnEnd 不会漏掉最后一段正文）。
            while let Some(first) = rx.recv().await {
                let mut batch = vec![first];
                let deadline = std::time::Instant::now() + std::time::Duration::from_millis(80);
                loop {
                    let rem = deadline.saturating_duration_since(std::time::Instant::now());
                    if rem.is_zero() {
                        break;
                    }
                    match tokio::time::timeout(rem, rx.recv()).await {
                        Ok(Some(ev)) => batch.push(ev),
                        _ => break,
                    }
                }
                let sid = run_target_sid();
                if sid.is_empty() {
                    continue;
                }
                let mut pending: Vec<&AgentEvent> = vec![];
                // move：信号句柄按值捕获（Copy），与结构事件臂对同名句柄的
                // 直接读写不撞借用；sid 传参（批内局部，不进闭包）。
                let mut flush_pending = move |pending: &[&AgentEvent], sid: &str| {
                    // 流式期间会话可能已被删除：事件照常消费，但消息不写
                    // 回，避免孤儿 entry
                    if !session_exists_in(space_sessions, sid) {
                        return;
                    }
                    // 读取与写回在同一把写锁内完成，避免跨锁的读后写窗口
                    let mut map = session_messages.write();
                    let mut ui = UiState {
                        messages: map.get(sid).cloned().unwrap_or_default(),
                    };
                    for ev in pending {
                        ui.apply(ev);
                    }
                    map.insert(sid.to_string(), ui.messages);
                };
                for ev in &batch {
                    if matches!(ev, AgentEvent::AssistantText { .. }) {
                        total_out += 1;
                    }
                    match ev {
                        AgentEvent::TurnStart
                        | AgentEvent::TurnEnd { .. }
                        | AgentEvent::ToolResult { .. } => {
                            // 先结算其前的高频 delta，再处理结构事件
                            if !pending.is_empty() {
                                flush_pending(&pending, &sid);
                                pending.clear();
                            }
                            match ev {
                                AgentEvent::TurnStart => {
                                    // 会话进入运行态（孤儿会话不写回）
                                    if session_exists_in(space_sessions, &sid) {
                                        let mut map = space_sessions.read().clone();
                                        for list in map.values_mut() {
                                            for s in list.iter_mut() {
                                                if s.id == sid {
                                                    s.status = SessionStatus::Active;
                                                    s.last_active = "刚刚".into();
                                                }
                                            }
                                        }
                                        space_sessions.set(map);
                                    }
                                }
                                AgentEvent::TurnEnd { .. } => {
                                    // TurnEnd：占位文案 + 最终文本持久化 + 状态收尾。
                                    // 会话已删除则跳过消息写回与会话状态更新（写回去
                                    // 等于复活已删会话），但 statusline 结算与
                                    // 会话回 Idle 必须照常执行。
                                    let deleted = !session_exists_in(space_sessions, &sid);
                                    if !deleted {
                                        let mut map = session_messages.write();
                                        let mut ui = UiState {
                                            messages: map.get(&sid).cloned().unwrap_or_default(),
                                        };
                                        ui.apply(ev);
                                        // 最后一条 assistant 消息即最终回复文本，连同其
                                        // 工具调用卡持久化到 daemon（线程内；空文本跳过，
                                        // 存储侧拒绝空消息）。tool_calls 一并落库，历史
                                        // 重载才能重建「工作过程」折叠区。
                                        let final_msg = ui
                                            .messages
                                            .iter()
                                            .rev()
                                            .find(|m| m.role == "assistant")
                                            .cloned();
                                        let final_text = final_msg
                                            .as_ref()
                                            .map(|m| m.content.clone())
                                            .unwrap_or_default();
                                        let final_tool_calls: Vec<serde_json::Value> = final_msg
                                            .map(|m| {
                                                m.tool_calls
                                                    .iter()
                                                    .filter_map(|tc| serde_json::to_value(tc).ok())
                                                    .collect()
                                            })
                                            .unwrap_or_default();
                                        if !final_text.is_empty() {
                                            let sid_daemon = sid.clone();
                                            let d_turn = d_consumer.clone();
                                            std::thread::spawn(move || {
                                                let _ = d_turn.append_message(
                                                    &sid_daemon,
                                                    false,
                                                    &final_text,
                                                    &[],
                                                    &final_tool_calls,
                                                );
                                            });
                                        }
                                        map.insert(sid.clone(), ui.messages);
                                    }

                                    let mut st = statusline();
                                    st.tokens_out += total_out;
                                    st.tokens_in = (session_messages
                                        .read()
                                        .get(&sid)
                                        .map(|list| {
                                            list.iter().map(|m| m.content.len()).sum::<usize>()
                                        })
                                        .unwrap_or(0)
                                        / 4)
                                        as u64;
                                    st.cost_usd += total_out as f64 * 0.000002;
                                    st.context_pct = ((st.tokens_in + st.tokens_out) as f64
                                        / st.context_max as f64
                                        * 100.0)
                                        .min(100.0);
                                    // WP-C（ROADMAP 5.6）：turn 计时结算——on_send 起的
                                    // 计时在这里落成总耗时，状态行改显示已结束耗时。
                                    // 无在飞 run 时 finish_run 是 no-op，不会清掉上一轮。
                                    st.finish_run(now_ms());
                                    statusline.set(st);
                                    total_out = 0;

                                    if !deleted {
                                        let mut map = space_sessions.read().clone();
                                        for list in map.values_mut() {
                                            for s in list.iter_mut() {
                                                if s.id == sid {
                                                    s.status = SessionStatus::Idle;
                                                }
                                            }
                                        }
                                        space_sessions.set(map);
                                    }
                                    // run 收尾，在飞 run id 清空（WP-C：之后列表状态
                                    // 以 run_status_cache 的持久记录为准）
                                    live_run_id.set(String::new());
                                    // 看板刷新兜底：ToolResult 臂已按工具名 bump，这里
                                    // 对每个 turn 收尾再 bump 一次——WireTranslator 的
                                    // LIFO 配对会丢未配对 end（convert.rs），只挂
                                    // ToolResult 会漏刷新，双触发防漏
                                    board_version.set(board_version() + 1);
                                }
                                AgentEvent::ToolResult { name, .. } => {
                                    // 看板刷新：todo/goal 的唯一写入口是模型工具（看板
                                    // 无编辑 RPC），命中这五个工具名即 todos.jsonl /
                                    // goals.jsonl 可能已变 → bump 版本号，WP-C 看板
                                    // effect 依赖表读它即重查。未命中的工具名不 bump，
                                    // 无关工具每跑一次都重查看板只是空耗 UDS 往返。
                                    if matches!(
                                        name.as_str(),
                                        "todo_add"
                                            | "todo_update"
                                            | "todo_list"
                                            | "goal_add"
                                            | "goal_link"
                                    ) {
                                        board_version.set(board_version() + 1);
                                    }
                                    if !session_exists_in(space_sessions, &sid) {
                                        continue;
                                    }
                                    let mut map = session_messages.write();
                                    let mut ui = UiState {
                                        messages: map.get(&sid).cloned().unwrap_or_default(),
                                    };
                                    ui.apply(ev);
                                    map.insert(sid.clone(), ui.messages);
                                }
                                _ => unreachable!("structural arm matched non-structural"),
                            }
                        }
                        // 高频 delta：挂起攒窗，窗尾一次性落库
                        _ => pending.push(ev),
                    }
                }
                if !pending.is_empty() {
                    flush_pending(&pending, &sid);
                }
            }
        });
        w_pipeline_started.set(true);
    });
    // known-issue 1：Disconnected 不是单向闸。挂载时探测只有 2s 有界预算，
    // daemon 冷启动 / 重启窗口内一旦落锁为 Disconnected，之后所有发送都被
    // 静默丢弃（本地回显 + 复位 Idle）。Disconnected 期间每 3s 重探一次：
    // 探活成功切回 Daemon 并补拉项目 / 会话列表；订阅管线 effect 依赖
    // backend()，切回时自动重建（一次性守卫保证不多建）。
    let reprobe_data_dir = config.data_dir.clone();
    use_effect(move || {
        if matches!((backend)(), DataBackend::Daemon(_)) {
            return;
        }
        let (re_tx, mut re_rx) = tokio::sync::mpsc::unbounded_channel::<DataBackend>();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(3));
                match probe_backend() {
                    DataBackend::Daemon(d) => {
                        let _ = re_tx.send(DataBackend::Daemon(d));
                        return;
                    }
                    DataBackend::Disconnected => {}
                }
            }
        });
        let value = reprobe_data_dir.clone();
        spawn(async move {
            let data_dir = value;
            while let Some(found) = re_rx.recv().await {
                let DataBackend::Daemon(d) = found else {
                    continue;
                };
                backend.set(DataBackend::Daemon(d.clone()));
                let space = daemon_space(&data_dir);
                spaces.set(vec![space.clone()]);
                active_space_path.set(space.path.clone());
                let list = std::thread::spawn(move || d.list_sessions(50).unwrap_or_default())
                    .join()
                    .unwrap_or_default();
                space_sessions.set(HashMap::from([(space.path, list.clone())]));
                active_session_id.set(list.first().map(|s| s.id.clone()).unwrap_or_default());
                break;
            }
        });
    });

    // known-issue 2：刷新后标题补重试。pending_titles 仅内存态，页面刷新后
    // 首条消息标题落库失败过的会话永远停在「会话 <ts>」占位。挂载（含重探
    // 切回后的补拉）时对占位会话按首条用户消息重新派生并经 daemon 写回：
    // 上限 5 个会话、每个两条 RPC 顺序执行；消息为空的会话无首条可派生，
    // 保持占位（那是真实状态，不编造标题）。
    use_effect(move || {
        let DataBackend::Daemon(d) = backend() else {
            return;
        };
        let sids: Vec<String> = space_sessions()
            .values()
            .flatten()
            .filter(|s| is_placeholder_title(&s.title))
            .map(|s| s.id.clone())
            .take(5)
            .collect();
        if sids.is_empty() {
            return;
        }
        let (rep_tx, mut rep_rx) = tokio::sync::mpsc::unbounded_channel::<(String, String)>();
        std::thread::spawn(move || {
            for sid in sids {
                let Ok(msgs) = d.load_messages(&sid, 50) else {
                    continue;
                };
                let Some(first) = msgs.iter().find(|m| m.role == "user") else {
                    continue;
                };
                let title = title_from_first_message(&first.content);
                if d.update_session_title(&sid, &title).is_err() {
                    continue;
                }
                let _ = rep_tx.send((sid, title));
            }
        });
        spawn(async move {
            while let Some((sid, title)) = rep_rx.recv().await {
                // 写回前重读：会话可能并行被删，孤儿 entry 不写回（同
                // session_exists_in 的纪律）；本地标题已不再占位（如用户
                // 期间改名）也不覆盖
                let mut map = space_sessions.read().clone();
                let mut patched = false;
                for list in map.values_mut() {
                    for s in list.iter_mut() {
                        if s.id == sid && is_placeholder_title(&s.title) {
                            s.title = title.clone();
                            patched = true;
                        }
                    }
                }
                if patched {
                    space_sessions.set(map);
                }
            }
        });
    });

    // WP-C：会话列表状态推断。只在 Daemon 模式跑（无 daemon 无 run 记录）；
    // 列表数据变化才重查——缓存已覆盖当前全部会话 id 即跳过，避免每次
    // 渲染都重复请求。查询放 spawn 里不阻塞首帧；≤50 会话 × 一次 UDS
    // 往返，耗时可忽略。
    use_effect(move || {
        let DataBackend::Daemon(d) = backend() else {
            return;
        };
        let sids: HashSet<String> = space_sessions
            .read()
            .values()
            .flatten()
            .map(|s| s.id.clone())
            .collect();
        let cached: HashSet<String> = run_status_cache.read().keys().cloned().collect();
        if sids == cached {
            return;
        }
        let live = live_run_id();
        spawn(async move {
            let live_opt: Option<&str> = if live.is_empty() {
                None
            } else {
                Some(live.as_str())
            };
            let mut map: HashMap<String, SessionStatus> = HashMap::new();
            for sid in &sids {
                let runs = d.runs_for_session(sid, 100).unwrap_or_default();
                map.insert(sid.clone(), infer_session_status(&runs, live_opt));
            }
            run_status_cache.set(map);
        });
    });

    // WP-C：任务看板数据源 = 当前会话的真实 run 记录 + 三份持久存储：
    // CLI 任务（tasks.jsonl）、模型工具写入的 todo（todos.jsonl）与 goal
    // （goals.jsonl）。run 记录是当前会话的瞬时执行，三份存储是跨会话的
    // 持久记录，都是看板的诚实内容；`kymido task add/done/...` 与 slice1 的
    // 模型工具写入的数据此前 web 完全看不到，这里通过 `task.list` /
    // `todo.list` / `goal.list` 补上。「任务系统」视图本身属于 C8（酒馆
    // 触发，已暂缓），不新造任务子系统，只把四份真实记录投影成任务卡
    // （`TaskItem::from_run` / `from_task` / `from_todo` / `from_goal`）。
    // 刷新时机：面板打开、切会话、run 起止（live_run_id 变化）、
    // todo/goal 工具写盘（board_version bump）——都不在渲染路径同步阻塞，
    // RPC 放 spawn 里。无 daemon（Disconnected）→ 看板恒空。
    let mut run_tasks: Signal<Vec<TaskItem>> = use_signal(Vec::new);
    let mut store_tasks: Signal<Vec<TaskItem>> = use_signal(Vec::new);
    use_effect(move || {
        // 依赖登记：面板关闭时不查（省一次 UDS 往返）；run 起止、todo/goal
        // 工具结果（board_version bump，见事件消费处）触发重查。值本身不
        // 参与查询参数，仅登记依赖。
        let open = show_tasks();
        let sid = active_session_id();
        let live = live_run_id();
        let _board_version = board_version();
        let DataBackend::Daemon(d) = backend() else {
            run_tasks.set(Vec::new());
            store_tasks.set(Vec::new());
            return;
        };
        if !open || sid.is_empty() {
            return;
        }
        spawn(async move {
            // RPC 失败不静默吞：TaskPanel 是用户主动展开的面板，一次瞬时
            // daemon 错误若被 unwrap_or_default() 抹掉，用户只会看到空列表
            // 且无从排查。降级仍是空列表（不 panic、不阻塞渲染），但留下日志。
            let runs = match d.runs_for_session(&sid, RUN_TASK_LIMIT) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("[web] runs_for_session({sid}) failed: {e}");
                    Vec::new()
                }
            };
            // 任务存储是跨会话的持久编排，不按会话过滤：取最近更新的若干条。
            // 降级形态与 runs 分支一致（eprintln! 留痕 + 空列表），不静默吞。
            let tasks = match d.task_list(TASK_BOARD_LIMIT) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("[web] task_list failed: {e}");
                    Vec::new()
                }
            };
            // todo / goal 存储（slice1 模型工具的唯一落盘点）：与 task 存储
            // 同型——limit 可选、降级同形态（eprintln! 留痕 + 空列表）。
            // 文件不存在时 daemon 返 `[]`（slice2 契约），不是错误。
            let todos = match d.todo_list(TASK_BOARD_LIMIT) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("[web] todo_list failed: {e}");
                    Vec::new()
                }
            };
            let goals = match d.goal_list(TASK_BOARD_LIMIT) {
                Ok(g) => g,
                Err(e) => {
                    eprintln!("[web] goal_list failed: {e}");
                    Vec::new()
                }
            };
            // 落后于用户操作的结果丢弃：查询期间切了会话、或起了/结束了 run，
            // 先返回的那次 RPC 会拿旧会话的记录覆盖当前看板。effect 每次依赖
            // 变化都重发一次新查询，丢掉陈旧结果只损失一次已无用的往返。
            // 不校验 open：面板关闭时看板本就不渲染，写进去也无害。
            if active_session_id() != sid || live_run_id() != live {
                return;
            }
            run_tasks.set(
                runs.iter()
                    .map(|r| {
                        TaskItem::from_run(
                            &r.run_id,
                            r.started_at_ms,
                            r.finished_at_ms,
                            r.status.as_deref(),
                        )
                    })
                    .collect(),
            );
            // 合并顺序 tasks → todos → goals：每份列表内部保持 daemon 返回
            // 序（各自按 updated_at 降序），跨列表不再重排——三种来源混在
            // 一个看板里，统一重排会假造新旧关系。
            let mut board: Vec<TaskItem> = tasks.iter().map(TaskItem::from_task).collect();
            board.extend(todos.iter().map(TaskItem::from_todo));
            board.extend(goals.iter().map(TaskItem::from_goal));
            store_tasks.set(board);
        });
    });

    let mut sidebar_collapsed = SIDEBAR_COLLAPSED.signal();

    let active_space = spaces()
        .iter()
        .find(|s| s.path == active_space_path())
        .cloned()
        .unwrap_or_else(|| WorkspaceSpace {
            id: String::new(),
            name: "kymido".into(),
            path: String::new(),
            branch: String::new(),
            is_active: true,
        });
    let active_title = space_sessions
        .read()
        .values()
        .flatten()
        .find(|s| s.id == active_session_id())
        .map(|s| s.title.clone())
        .unwrap_or_else(|| "新会话".into());

    let current_messages = session_messages()
        .get(&active_session_id())
        .cloned()
        .unwrap_or_default();

    // ⌘K 候选列表：Daemon 消费 search_sessions 的结果（空 query 时 effect
    // 已列 daemon 全量）；无 daemon 时没有可检索的会话来源 → 空候选。
    let mut quick_switcher_list = match backend() {
        DataBackend::Daemon(_) => switcher_sessions(),
        DataBackend::Disconnected => Vec::new(),
    };
    // WP-C：列表状态喂入——在飞（Active）优先，其次 run 记录推断出的
    // 缓存状态（Aborted/Idle），最后会话自身状态。无 daemon 时缓存恒空，
    // 归一不变。
    apply_run_statuses(&mut quick_switcher_list, &run_status_cache());

    // 各 move 闭包各自的 config 克隆（LlmRuntimeConfig 非 Copy；render 侧
    // 仍读 config.image_input，闭包不能把它 move 走）。
    let config_create = config.clone();
    let config_model = config.clone();
    let config_send = config.clone();

    // 共享信号集（R3：动作层具名 fn 一次收齐 Signal，闭包不再逐捕获）。
    let sig = WorkspaceSignals {
        spaces,
        active_space_path,
        space_sessions,
        active_session_id,
        session_messages,
        pending_titles,
        statusline,
        pending_question,
        show_quick_switcher,
        show_tasks,
        view,
        backend,
        readiness,
        run_target_sid,
        live_run_id,
        board_version,
    };

    // ── 数据操作（内存即时生效；Daemon 模式再追加 daemon 持久化）──────────

    let on_select_space = move |path: String| {
        select_space(sig, path);
    };

    let on_create_session = move |space_path: String| {
        create_session(sig, config_create.model.clone(), space_path);
    };

    let on_archive_session = move |id: String| {
        archive_session(sig, id);
    };

    let on_delete_space = move |space_path: String| {
        delete_space(sig, space_path);
    };
    // A2：打开「创建项目」目录弹窗（路径草稿清空，用户输入目录）。
    let on_open_create_project = move |_| {
        create_project_path.set(String::new());
        show_create_project.set(true);
    };

    let on_model_change = move |m: String| {
        change_model(sig, config_model.clone(), m, on_update_config);
    };

    let on_abort = move |()| {
        abort_run(sig);
    };

    // 回答问题（plan-mode review 卡片）：RPC 在线程里跑，结果经 channel 回
    // 消费侧清卡片（Signal 非 Send，不能进 std::thread——同 worker 管线）。
    // 终局错误（question_not_found / question_already_answered：问题已从
    // broker 消失，重试永远失败）也清卡片；只有传输类错误保留等重试。
    let on_answer = move |(qid, answer): (String, QuestionAnswer)| {
        answer_question(sig, qid, answer);
    };

    let on_thinking_change = move |level: String| {
        set_thinking(sig, level);
    };

    // ── 发送：内存即时上屏 + mock 模拟流；Daemon 模式追加持久化 ───────────

    let on_send = move |(text, attachments): (String, Vec<PendingAttachment>)| {
        send_message(sig, config_send.clone(), text, attachments);
    };

    // ── 渲染 ────────────────────────────────────────────────────────────────

    // WP-C：侧栏会话列表喂入推断出的状态（在飞 Active 优先，其次 run 记录
    // 推断的缓存状态，最后会话自身状态）。Mock 无缓存，合并后与原样一致。
    let sidebar_sessions = merge_run_statuses(space_sessions.read().clone(), &run_status_cache());

    // 顶栏折叠钮 aria-label（同 title 语义）：rsx 字符串插值不接受 block
    // if/else（dioxus formatted segment 只解析 Ident/表达式），前置算好，
    // 同 ui-kit NavTopBar 的 toggle_label 模式。
    let collapse_aria = if sidebar_collapsed() {
        sh::BTN_EXPAND_SIDEBAR
    } else {
        sh::BTN_COLLAPSE_SIDEBAR
    };
    rsx! {
        // 三列框架壳在 layouts/app_frame.rs：这里只喂侧栏、中栏头与正文。
        AppFrame {
            collapsed: sidebar_collapsed(),
            sidebar: rsx! {
            // 侧栏点击也视为「面板外」→ 关闭任务看板（ainnotation 波3 #5）。
            // display:contents 不生成盒子，aside 仍是 AppFrame grid 直属
            // 子项，布局零变化；onclick 靠冒泡接住侧栏内点击。
            div { class: "contents",
                onclick: move |_| {
                    if show_tasks() {
                        show_tasks.set(false);
                    }
                },
                DualSidebar {
                    spaces: spaces(),
                    space_sessions: sidebar_sessions,
                    active_id: active_session_id(),
                    on_select: move |id: String| {
                        active_session_id.set(id);
                        view.set(View::Chat);
                    },
                    on_select_space: on_select_space,
                    on_create: on_create_session,
                    on_archive_session: on_archive_session,
                    on_delete_space: on_delete_space,
                    on_open_search: move |_| show_quick_switcher.set(true),
                    on_create_project: on_open_create_project,
                    view: view,
                    settings_section: settings_section,
                    expanded: !sidebar_collapsed(),
                }
            }
            },
            // 中栏：面包屑头 + 视图
            header: rsx! {
                // 中栏头（面包屑）点击也视为「面板外」→ 关闭任务看板。
                // 头 div 在本页（workspace.rs）撰写、作为 AppFrame 的 header 槽
                // 传入，故无需改 app_frame.rs 即可覆盖「其他非面板区域」。
                div { class: "min-h-[44px] pl-2 pr-5 pt-2.5 pb-2 border-b border-border flex items-center gap-2 shrink-0",
                    onclick: move |_| {
                        if show_tasks() {
                            show_tasks.set(false);
                        }
                    },
                    // 顶层栏最左折叠钮（2026-10 批注轮：从侧栏 top 槽移到顶栏最左）。
                    // 按钮结构跟 ui-kit NavTopBar 最左钮（data-testid / ANIM_SCOPE /
                    // hover 面）；但 kymido 中栏头中间是面包屑，NavTopBar 的
                    // justify-between 会把面包屑推到右缘，故只复用其按钮 markup。
                    // 图标按展开态切换（动态）：展开 = chevron-left（收起方向），
                    // 折叠 = chevron-right（展开方向），两枚都是 ui-kit icon-anim
                    // hover 微动效图标。
                    button {
                        "data-testid": "sidebar-collapse",
                        "aria-label": "{collapse_aria}",
                        class: "hidden h-7 w-7 shrink-0 items-center justify-center rounded-lg text-muted-foreground hover:bg-card hover:text-foreground md:flex {ANIM_SCOPE}",
                        onclick: move |_| {
                            sidebar_collapsed.set(!sidebar_collapsed());
                        },
                        if sidebar_collapsed() {
                            IconChevronRight { size: 16 }
                        } else {
                            IconChevronLeft { size: 16 }
                        }
                    }
                    if view() == View::Stats {
                        span { class: "role-hint font-medium text-foreground", "{sh::TTL_STATS}" }
                    } else if view() == View::Archive {
                        span { class: "role-hint font-medium text-foreground", "{sh::LBL_ARCHIVE}" }
                    } else if view() == View::Settings {
                        span { class: "role-hint font-medium text-foreground", "设置" }
                    } else {
                        span { class: "role-hint font-medium text-foreground", "{active_space.name}" }
                        if !active_space.branch.is_empty() {
                            span { class: "text-muted-foreground", "/" }
                            span { class: "role-hint truncate max-w-[360px]", "{active_title}" }
                            span { class: "font-mono role-label px-2 py-0.5 rounded-full bg-chip-brand text-brand-300 border border-border shrink-0",
                                "{active_space.branch}"
                            }
                        }
                    }
                    div { class: "flex-1" }
                    if active_session_running(space_sessions, active_session_id) {
                        span { class: "flex items-center gap-1.5 role-caption",
                            ui_kit::Spinner { size: 10, class: "text-brand" }
                            "运行中"
                        }
                    }
                }
            },
            // 中栏正文：统计页 / 会话页
            children: rsx! {
                match view() {
                    View::Stats => rsx! { StatsView { range: stats_range } },
                    View::Archive => rsx! {
                        ArchiveView { rev: archive_rev }
                    },
                    View::Settings => rsx! {
                        SettingsPage {
                            section: settings_section,
                            config: config.clone(),
                            on_update_config: on_update_config,
                        }
                    },
                    View::Chat => rsx! {
                        Chat {
                            messages: current_messages,
                            statusline: statusline(),
                            // T5：当前模型不声明 image 输入时，composer 附件入口置灰。
                            image_input: config.image_input,
                            is_streaming: active_session_running(space_sessions, active_session_id),
                            on_send: on_send,
                            on_model_change: on_model_change,
                            on_thinking_change: on_thinking_change,
                            on_abort: on_abort,
                            question: pending_question(),
                            on_answer: on_answer,
                            on_toggle_tasks: move |_| show_tasks.set(!show_tasks()),
                            on_outside_tasks: move |_| {
                                if show_tasks() {
                                    show_tasks.set(false);
                                }
                            },
                            dock: show_tasks().then(|| rsx! {
                                TaskPanel {
                                    // 持久编排（tasks.jsonl）在前，瞬时执行（run 记录）随后；
                                    // 合并只在渲染处做，不引入第三个 signal 缓存中间结果
                                    tasks: {
                                        let mut board = store_tasks();
                                        board.extend(run_tasks());
                                        board
                                    },
                                    on_close: move |_| show_tasks.set(false),
                                }
                            }),
                        }
                    },
                }

            // ⌘K 快速切换
            if show_quick_switcher() {
                Modal { width_class: "w-[560px]", top_aligned: true, on_close: move |_| show_quick_switcher.set(false),
                    div { class: "p-3 flex flex-col gap-2",
                        input {
                            class: "w-full h-11 rounded-xl bg-card border border-border px-4 role-hint text-foreground outline-none transition-colors focus:border-brand placeholder:text-muted-foreground",
                            r#type: "text",
                            placeholder: "搜索会话名称或编号...",
                            value: "{search_query}",
                            oninput: move |e| search_query.set(e.value().clone()),
                            onkeydown: move |e: KeyboardEvent| {
                                if e.key() == Key::Escape {
                                    show_quick_switcher.set(false);
                                }
                            },
                            autofocus: true,
                        }
                        div { class: "max-h-[320px] overflow-y-auto flex flex-col gap-px",
                            for session in quick_switcher_list.iter()
                            {
                                {
                                    let id = session.id.clone();
                                    let is_active = session.id == active_session_id();
                                    let row_class = if is_active {
                                        "h-10 px-3 rounded-[10px] flex items-center justify-between cursor-pointer transition-colors bg-muted"
                                    } else {
                                        "h-10 px-3 rounded-[10px] flex items-center justify-between cursor-pointer transition-colors hover:bg-muted"
                                    };
                                    let dot = match session.status {
                                        SessionStatus::Active => "bg-brand",
                                        SessionStatus::Archived | SessionStatus::Aborted => "bg-destructive/70",
                                        SessionStatus::Idle => "bg-dim",
                                    };
                                    rsx! {
                                        div { class: "{row_class}",
                                            onclick: move |_| {
                                                active_session_id.set(id.clone());
                                                view.set(View::Chat);
                                                show_quick_switcher.set(false);
                                            },
                                            div { class: "flex items-center gap-2.5 min-w-0",
                                                span { class: "w-2 h-2 rounded-full {dot} shrink-0" }
                                                span { class: "role-hint text-foreground truncate", "{session.title}" }
                                            }
                                            span { class: "font-mono role-label shrink-0", "{session.id}" }
                                        }
                                    }
                                }
                            }
                        }
                        div { class: "pt-2 border-t border-border flex items-center justify-between role-caption",
                            span { "选择会话快速切换" }
                            div { class: "flex items-center gap-1.5",
                                kbd { "ESC" }
                                span { "退出" }
                            }
                        }
                    }
                }
            }
            // A2 创建项目：目录弹窗（输入已存在目录路径，注册为项目）
            if show_create_project() {
                Modal {
                    width_class: "w-[560px]",
                    top_aligned: true,
                    on_close: move |_| show_create_project.set(false),
                    div { class: "p-4 flex flex-col gap-3",
                        div { class: "flex flex-col gap-1",
                            label { class: "role-hint font-medium text-foreground", "创建项目（目录路径）" }
                            p { class: "role-caption", "输入一个已存在的目录路径，注册为新项目。同一路径重复注册会被拒绝。" }
                        }
                        input {
                            class: "w-full h-11 rounded-xl bg-card border border-border px-4 role-hint text-foreground outline-none transition-colors focus:border-brand placeholder:text-muted-foreground",
                            r#type: "text",
                            placeholder: "~/projects/your-repo",
                            value: "{create_project_path}",
                            oninput: move |e| create_project_path.set(e.value().clone()),
                            onkeydown: move |e: KeyboardEvent| {
                                if e.key() == Key::Enter {
                                    let path = create_project_path().clone();
                                    add_space(sig, path);
                                    show_create_project.set(false);
                                } else if e.key() == Key::Escape {
                                    show_create_project.set(false);
                                }
                            },
                            autofocus: true,
                        }
                        div { class: "flex justify-end gap-2",
                            button {
                                class: "h-9 px-4 rounded-lg role-caption hover:bg-secondary-hover transition-colors cursor-pointer border-none bg-transparent",
                                onclick: move |_| show_create_project.set(false),
                                "取消"
                            }
                            button {
                                class: "h-9 px-4 rounded-lg role-caption font-medium text-primary-foreground bg-brand hover:opacity-90 transition-opacity cursor-pointer border-none",
                                onclick: move |_| {
                                    let path = create_project_path().clone();
                                    add_space(sig, path);
                                    show_create_project.set(false);
                                },
                                "创建"
                            }
                        }
                    }
                }
            }
            },
        }
    }
}
