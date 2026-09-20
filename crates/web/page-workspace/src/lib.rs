//! 工作区页：dsh AppFrame 三列布局（侧栏 280 / 中栏）。
//!
//! 唯一数据源是本机 daemon（G5 删 mock）：启动时探测 + ping，连不上就是
//! 空态——空项目列表、空会话列表、空状态行、空任务看板，既不回退假数据
//! 也不 panic。assistant 流走 daemon `event.subscribe`（`worker_event_loop`
//! 读线程 → channel → 消费 task）。

use std::collections::{HashMap, HashSet};

use dioxus::prelude::*;
use omenic_web_client::daemon::WebDaemon;
use omenic_web_client::llm::LlmRuntimeConfig;
use omenic_web_components::chat::Chat;
use omenic_web_components::sidebar::Sidebar;
use omenic_web_components::taskpanel::TaskPanel;
use omenic_web_components::ui::Modal;
use omenic_web_page_config::SettingsModal;
use omenic_web_page_stats::StatsView;
use omenic_web_state::convert::{WireTranslator, infer_session_status};
use omenic_web_state::title_from_first_message;
use omenic_web_state::types::{
    ChatMessage, Session, SessionStatus, StatusLine, TaskItem, WorkspaceSpace,
};
use omenic_web_state::ui_state::{AgentEvent, UiState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Chat,
    Stats,
}

/// 任务看板拉取的 run 记录条数上限（与会话状态推断的 100 同量级；
/// run ledger 是追加日志，取最近若干条足够铺满看板）。
const RUN_TASK_LIMIT: u32 = 50;

/// 任务看板拉取的任务存储条数上限（daemon `task.list` 的缺省值一致：
/// 按 `updated_at` 降序取最近若干条，跨会话的持久编排全在此列）。
const TASK_BOARD_LIMIT: u32 = 50;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// 数据后端：唯一真实来源是本机 omenic daemon。初始化连接失败
/// （无 daemon / ping 不通）→ [`DataBackend::Disconnected`]，全部数据源
/// 返回空集合（空态），不回退假数据、不 panic。
#[derive(Debug, Clone)]
enum DataBackend {
    Daemon(WebDaemon),
    /// daemon 不可达：项目/会话/消息/任务一律空态。
    Disconnected,
}

/// Daemon 模式下的唯一真实项目行：名字取 data_dir 的文件名。
fn daemon_space(data_dir: &str) -> WorkspaceSpace {
    let name = std::path::Path::new(data_dir)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "omenic".into());
    WorkspaceSpace {
        id: data_dir.to_string(),
        name,
        path: data_dir.to_string(),
        branch: String::new(),
        is_active: true,
    }
}

/// 新建会话（内存版；Daemon 模式下调用方再追加 daemon 持久化）。
/// 独立成自由函数：Signal 是 Copy，任意闭包都可以直接调用，避免处理器
/// 闭包被多处 move。返回 (会话 id, 标题) 供 daemon 侧 create 使用。
/// 标题是 `会话 <ts>` 时间戳占位——侧栏新建瞬间先占位，首条用户消息
/// 发出后由 `on_send` 换成 [`title_from_first_message`] 的截词标题。
#[allow(clippy::too_many_arguments)]
fn create_session_in(
    space_path: String,
    model: String,
    mut space_sessions: Signal<HashMap<String, Vec<Session>>>,
    mut session_messages: Signal<HashMap<String, Vec<ChatMessage>>>,
    mut active_space_path: Signal<String>,
    mut active_session_id: Signal<String>,
) -> (String, String) {
    let ts = now_ms();
    let new_id = format!("s-{}", ts);
    let title = format!("会话 {}", ts % 1_000_000);
    let new_session = Session {
        id: new_id.clone(),
        title: title.clone(),
        last_active: "刚刚".into(),
        model,
        status: SessionStatus::Idle,
        last_active_epoch: ts,
        parent_id: None,
    };
    let mut map = space_sessions.read().clone();
    let mut list = map.get(&space_path).cloned().unwrap_or_default();
    list.insert(0, new_session);
    map.insert(space_path.clone(), list);
    space_sessions.set(map);
    session_messages.write().insert(new_id.clone(), Vec::new());
    active_space_path.set(space_path);
    active_session_id.set(new_id.clone());
    (new_id, title)
}

/// 新建子会话（内存版；Daemon 模式下调用方再追加 daemon 持久化）。与
/// [`create_session_in`] 的根会话路径（insert(0)）互补：子会话插在父会话
/// **之后**——侧栏树里子要紧跟父，而不是顶到列表头。`parent_id` 指向
/// 父 id；父已不在任何 space（刚被删 / 换 space 的竞态）→ 不挂悬空的
/// parent_id，退化为根会话插在激活 space 的列表头（侧栏分组对悬空父
/// 同样当根处理，内存与之一致）。
#[allow(clippy::too_many_arguments)]
fn create_child_session_in(
    parent_id: String,
    model: String,
    mut space_sessions: Signal<HashMap<String, Vec<Session>>>,
    mut session_messages: Signal<HashMap<String, Vec<ChatMessage>>>,
    mut active_space_path: Signal<String>,
    mut active_session_id: Signal<String>,
) -> (String, String) {
    let ts = now_ms();
    let new_id = format!("s-{}", ts);
    let title = format!("会话 {}", ts % 1_000_000);
    let mut map = space_sessions.read().clone();
    // 会话 id 跨 space 唯一，命中第一个即可；返回 (space, 父在列表内的下标)
    let parent_pos = map.iter().find_map(|(path, list)| {
        list.iter()
            .position(|s| s.id == parent_id)
            .map(|p| (path.clone(), p))
    });
    let parent_id_field = if parent_pos.is_some() {
        Some(parent_id)
    } else {
        None
    };
    let new_session = Session {
        id: new_id.clone(),
        title: title.clone(),
        last_active: "刚刚".into(),
        model,
        status: SessionStatus::Idle,
        last_active_epoch: ts,
        parent_id: parent_id_field,
    };
    match parent_pos {
        Some((path, pos)) => {
            map.get_mut(&path).unwrap().insert(pos + 1, new_session);
            session_messages.write().insert(new_id.clone(), Vec::new());
            active_space_path.set(path);
        }
        None => {
            let path = active_space_path();
            map.entry(path.clone()).or_default().insert(0, new_session);
            session_messages.write().insert(new_id.clone(), Vec::new());
            active_space_path.set(path);
        }
    }
    space_sessions.set(map);
    active_session_id.set(new_id.clone());
    (new_id, title)
}

/// 会话是否仍存在于任一 space。流式期间会话可能被用户删除
/// （`on_delete_session` 会同时清掉消息），孤儿 entry 不能写回。
fn session_exists_in(space_sessions: Signal<HashMap<String, Vec<Session>>>, sid: &str) -> bool {
    space_sessions
        .read()
        .values()
        .flatten()
        .any(|s| s.id == sid)
}

/// 订阅读线程：阻塞消费 `event.subscribe` 推送帧，断线退避重连
/// （1s/2s/4s/5s 封顶）。只拥有 `WebDaemon` 克隆、`Subscription`、
/// `WireTranslator` 与 `tx`——不碰任何 Signal（use_signal 底层
/// UnsyncStorage 非 Send，不能进 std::thread）。退出条件：`tx.send`
/// 失败（消费端随组件卸载而亡）或 keepalive tick 感知 `tx.is_closed`。
fn worker_event_loop(d: WebDaemon, tx: tokio::sync::mpsc::UnboundedSender<AgentEvent>) {
    use std::time::Duration;
    const BACKOFF: [u64; 4] = [1, 2, 4, 5];
    let mut attempt = 0usize;
    loop {
        if let Ok(mut sub) = d.subscribe_worker() {
            attempt = 0;
            let mut translator = WireTranslator::new();
            loop {
                // 5s keepalive：None 空转 tick 顺带感知消费端死亡，把组件
                // 卸载后读线程的残留窗口从 30s 压到 5s（空转只是一次
                // syscall，开销可忽略）
                match sub.next_event(Duration::from_secs(5)) {
                    Ok(Some(frame)) => {
                        if let Some(ev) = translator.translate(&frame.event)
                            && tx.send(ev).is_err()
                        {
                            return; // 消费端已亡
                        }
                    }
                    Ok(None) => {
                        if tx.is_closed() {
                            return; // 消费端已亡：keepalive tick 时感知
                        }
                    }
                    Err(_) => break, // 断线 → 走重连
                }
            }
        }
        if tx.is_closed() {
            return;
        }
        eprintln!("retry in {}s", BACKOFF[attempt.min(BACKOFF.len() - 1)]);
        std::thread::sleep(Duration::from_secs(BACKOFF[attempt.min(BACKOFF.len() - 1)]));
        attempt += 1;
    }
}

#[component]
pub fn Workspace(
    config: LlmRuntimeConfig,
    on_update_config: EventHandler<LlmRuntimeConfig>,
) -> Element {
    // ── 数据后端：探测 daemon，连不上就是空态 ─────────────────────────────
    // 连接 + ping 放线程内执行（线程 + join，仿旧 db_load_sessions 的做法）；
    // UDS 往返耗时极短，不显著拖慢首帧。ping 不通（无 daemon / 陈旧 socket
    // 文件）→ None → Disconnected（全空态），不 panic、不阻塞渲染。
    let backend = use_signal(move || {
        // 探测线程的 panic 不静默吞：`.ok()` 会把 JoinError 抹成 Disconnected，
        // 于是 socket 解析或 ping 里的真 panic 只表现为空态，无从排查。
        // 失败仍是 Disconnected（空态），但先留下痕迹。
        match std::thread::spawn(|| WebDaemon::from_env_or_default().filter(|d| d.ping())).join() {
            Ok(Some(d)) => DataBackend::Daemon(d),
            Ok(None) => DataBackend::Disconnected,
            Err(e) => {
                eprintln!("[web] workspace daemon probe thread panicked: {e:?}");
                DataBackend::Disconnected
            }
        }
    });

    let data_dir = config.data_dir.clone();
    let mut spaces = use_signal(move || match backend() {
        // Daemon 模式：项目行 = 单行真实项目（名字取 data_dir 文件名）
        DataBackend::Daemon(_) => vec![daemon_space(&data_dir)],
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
        st.cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        st
    });
    let mut view = use_signal(|| View::Chat);
    let mut show_quick_switcher = use_signal(|| false);
    let mut show_settings = use_signal(|| false);
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
    let mut run_target_sid = use_signal(String::new);
    // WP-C：当前在飞 run 的 id（on_send 生成，随 worker.prompt 一起带给
    // daemon 记进 run ledger）。列表状态推断用它区分「正在跑」与「半开孤儿」；
    // TurnEnd / 中断时清空。空串 = 无在飞 run。
    let mut live_run_id = use_signal(String::new);
    // WP-C：会话列表状态推断缓存——daemon 的 SessionSummary 无状态字段，
    // 列表侧的 Idle/Active/Aborted 改由 run.list 的 run 记录组装。为避免
    // 每次渲染都重复请求，run 记录只在会话列表数据变化（加载/新建/删除）
    // 时重查一次；无 daemon 时不接 run.list，缓存恒空，渲染侧归一为原状态。
    let mut run_status_cache: Signal<HashMap<String, SessionStatus>> = use_signal(HashMap::new);
    use_effect(move || {
        let DataBackend::Daemon(d) = backend() else {
            return;
        };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
        let d_consumer = d.clone();
        std::thread::spawn(move || worker_event_loop(d, tx));
        spawn(async move {
            let mut total_out: u64 = 0;
            while let Some(ev) = rx.recv().await {
                let sid = run_target_sid();
                if sid.is_empty() {
                    continue;
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
                            ui.apply(&ev);
                            // 最后一条 assistant 消息即最终回复文本，持久化
                            // 到 daemon（线程内；空文本跳过，存储侧拒绝空
                            // 消息）
                            let final_text = ui
                                .messages
                                .iter()
                                .rev()
                                .find(|m| m.role == "assistant")
                                .map(|m| m.content.clone())
                                .unwrap_or_default();
                            if !final_text.is_empty() {
                                let sid_daemon = sid.clone();
                                let d_turn = d_consumer.clone();
                                std::thread::spawn(move || {
                                    let _ = d_turn.append_message(&sid_daemon, false, &final_text);
                                });
                            }
                            map.insert(sid.clone(), ui.messages);
                        }

                        let mut st = statusline();
                        st.tokens_out += total_out;
                        st.tokens_in = (session_messages
                            .read()
                            .get(&sid)
                            .map(|list| list.iter().map(|m| m.content.len()).sum::<usize>())
                            .unwrap_or(0)
                            / 4) as u64;
                        st.cost_usd += total_out as f64 * 0.000002;
                        st.context_pct =
                            ((st.tokens_in + st.tokens_out) as f64 / st.context_max as f64 * 100.0)
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
                    }
                    _ => {
                        if matches!(ev, AgentEvent::AssistantText { .. }) {
                            total_out += 1;
                        }
                        // 流式期间会话可能已被删除：事件照常消费，但消息
                        // 不写回，避免孤儿 entry
                        if !session_exists_in(space_sessions, &sid) {
                            continue;
                        }
                        // 读取与写回在同一把写锁内完成，避免跨锁的读后写
                        // 窗口
                        let mut map = session_messages.write();
                        let mut ui = UiState {
                            messages: map.get(&sid).cloned().unwrap_or_default(),
                        };
                        ui.apply(&ev);
                        map.insert(sid.clone(), ui.messages);
                    }
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

    // WP-C：任务看板数据源 = 当前会话的真实 run 记录 + CLI 任务存储
    // （tasks.jsonl）。run 记录是当前会话的瞬时执行，task 存储是跨会话
    // 的持久编排，两者都是看板的诚实内容；`oi task add/done/...` 写入的
    // 任务此前 web 完全看不到，这里通过 `task.list` 补上。「任务系统」
    // 视图本身属于 C8（酒馆触发，已暂缓），不新造任务子系统，只把两份
    // 真实记录投影成任务卡（`TaskItem::from_run` / `TaskItem::from_task`）。
    // 刷新时机：面板打开、切会话、run 起止（live_run_id 变化）——都不在
    // 渲染路径同步阻塞，RPC 放 spawn 里。无 daemon（Disconnected）
    // → 看板恒空。
    let mut run_tasks: Signal<Vec<TaskItem>> = use_signal(Vec::new);
    let mut store_tasks: Signal<Vec<TaskItem>> = use_signal(Vec::new);
    use_effect(move || {
        // 依赖登记：面板关闭时不查（省一次 UDS 往返），run 起止触发重查
        let open = show_tasks();
        let sid = active_session_id();
        let live = live_run_id();
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
            store_tasks.set(tasks.iter().map(TaskItem::from_task).collect());
        });
    });

    // 侧栏宽度/折叠：全局信号，切视图后仍保持
    let mut sidebar_collapsed = GlobalSignal::<bool>::new(|| false).signal();
    let mut sidebar_width = GlobalSignal::<usize>::new(|| 280).signal();
    let mut dragging = use_signal(|| false);
    let mut drag_start_x = use_signal(|| 0i32);
    let mut drag_start_width = use_signal(|| 280usize);
    let on_resize_start = move |x: i32| {
        dragging.set(true);
        drag_start_x.set(x);
        drag_start_width.set(sidebar_width());
    };
    let mut on_resize_move = move |e: MouseEvent| {
        // 自愈：拖拽中鼠标键已全部松开（mouseup 在窗口外被吞）→ 直接结束
        if e.held_buttons().is_empty() {
            dragging.set(false);
            return;
        }
        if dragging() {
            let raw =
                drag_start_width() as i32 + (e.client_coordinates().x as i32 - drag_start_x());
            sidebar_collapsed.set(raw < 100);
            sidebar_width.set((raw.max(100) as usize).min(420));
        }
    };
    let on_toggle_sidebar = move |_| sidebar_collapsed.set(!sidebar_collapsed());
    let on_expand_sidebar = move |_| {
        sidebar_width.set(280);
        sidebar_collapsed.set(false);
    };
    let mut preset_dragging = use_signal(|| false);
    let mut on_preset_move = move |e: MouseEvent| {
        if e.held_buttons().is_empty() {
            preset_dragging.set(false);
            return;
        }
        if preset_dragging() {
            let target = 56_i32 + e.client_coordinates().x as i32;
            sidebar_width.set(target.clamp(264, 420) as usize);
            sidebar_collapsed.set(false);
        }
    };
    let on_root_mousemove = move |e: MouseEvent| {
        on_resize_move(e.clone());
        on_preset_move(e);
    };
    let on_root_mouseup = move |_e: MouseEvent| {
        dragging.set(false);
        preset_dragging.set(false);
    };

    let active_space = spaces()
        .iter()
        .find(|s| s.path == active_space_path())
        .cloned()
        .unwrap_or_else(|| WorkspaceSpace {
            id: String::new(),
            name: "omenic".into(),
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

    // 各 move 闭包各自的 config 克隆（LlmRuntimeConfig 非 Copy）
    let config_create = config.clone();
    let config_model = config.clone();
    let config_send = config.clone();
    let config_child = config.clone();

    // ── 数据操作（内存即时生效；Daemon 模式再追加 daemon 持久化）──────────

    let on_select_space = move |path: String| {
        active_space_path.set(path);
    };

    let on_create_session = move |space_path: String| {
        let (new_id, title) = create_session_in(
            space_path,
            config_create.model.clone(),
            space_sessions,
            session_messages,
            active_space_path,
            active_session_id,
        );
        // Daemon 模式：内存建会话的同时持久化到 daemon（线程内）
        if let DataBackend::Daemon(d) = backend() {
            std::thread::spawn(move || {
                let _ = d.create_session(&new_id, &title);
            });
        }
        view.set(View::Chat);
    };

    // 创建子会话（5.3/5.5 谱系分组）：内存里插在父之后并挂 parent_id；
    // daemon 侧走带谱系边的 create_session_with_parent（线程内）
    let on_create_child = move |parent_id: String| {
        let (new_id, title) = create_child_session_in(
            parent_id.clone(),
            config_child.model.clone(),
            space_sessions,
            session_messages,
            active_space_path,
            active_session_id,
        );
        if let DataBackend::Daemon(d) = backend() {
            std::thread::spawn(move || {
                let _ = d.create_session_with_parent(&new_id, &title, Some(&parent_id));
            });
        }
        view.set(View::Chat);
    };

    let on_delete_session = move |id: String| {
        // Daemon 模式：daemon 侧删除（线程内，连带消息），内存清理照旧
        if let DataBackend::Daemon(d) = backend() {
            let id_daemon = id.clone();
            std::thread::spawn(move || {
                let _ = d.delete_session(&id_daemon);
            });
        }
        let mut map = space_sessions.read().clone();
        for list in map.values_mut() {
            list.retain(|s| s.id != id);
        }
        space_sessions.set(map);
        session_messages.write().remove(&id);
        if active_session_id() == id {
            let next = space_sessions
                .read()
                .get(&active_space_path())
                .and_then(|list| list.first().map(|s| s.id.clone()))
                .unwrap_or_default();
            active_session_id.set(next);
        }
    };

    let on_delete_space = move |space_path: String| {
        let mut sp = spaces();
        sp.retain(|s| s.path != space_path);
        spaces.set(sp);
        space_sessions.write().remove(&space_path);
        if active_space_path() == space_path {
            if let Some(first) = spaces().first() {
                active_space_path.set(first.path.clone());
            } else {
                active_space_path.set(String::new());
                active_session_id.set(String::new());
            }
        }
    };

    let on_model_change = move |m: String| {
        let mut st = statusline();
        st.model = m.clone();
        statusline.set(st);
        let mut cfg = config_model.clone();
        cfg.model = m;
        on_update_config.call(cfg);
    };

    let on_abort = {
        let backend_abort = backend;
        let mut space_sessions_abort = space_sessions;
        let mut live_run_id_abort = live_run_id;
        let mut statusline_abort = statusline;
        let sid_abort = active_session_id;
        move |()| {
            if let DataBackend::Daemon(d) = backend_abort() {
                std::thread::spawn(move || {
                    let _ = d.abort_worker();
                });
            }
            // 内存即时复位：会话回 Idle；事件流里的残余事件由孤儿守卫兜底。
            // 在飞 run id 也清空：中断后列表状态以 run ledger 的持久记录
            // 为准（WP-C）
            live_run_id_abort.set(String::new());
            // 计时结算（5.6）：中断后不会再有 TurnEnd，此处不结算耗时会一直
            // 按「在飞」实时增长。中断点即该 run 的终点。
            let mut st = statusline_abort();
            st.finish_run(now_ms());
            statusline_abort.set(st);
            let sid = sid_abort();
            let mut map = space_sessions_abort.read().clone();
            for list in map.values_mut() {
                for s in list.iter_mut() {
                    if s.id == sid {
                        s.status = SessionStatus::Idle;
                    }
                }
            }
            space_sessions_abort.set(map);
        }
    };

    let on_toggle_thinking = move |()| {
        let mut st = statusline();
        st.thinking = if st.thinking == "off" {
            "8k".into()
        } else {
            "off".into()
        };
        statusline.set(st);
    };

    // ── 发送：内存即时上屏 + mock 模拟流；Daemon 模式追加持久化 ───────────

    let on_send = move |text: String| {
        // 无会话时先建一个
        let mut created: Option<(String, String)> = None;
        if active_session_id().is_empty()
            || !space_sessions
                .read()
                .values()
                .any(|list| list.iter().any(|s| s.id == active_session_id()))
        {
            created = Some(create_session_in(
                active_space_path(),
                config_send.model.clone(),
                space_sessions,
                session_messages,
                active_space_path,
                active_session_id,
            ));
        }
        let sid = active_session_id();

        // 首条用户消息：标题从时间戳占位换成消息内容截词（确定性
        // fallback，不调 LLM）。以「内存消息列表此前为空」判定，而非
        // 匹配占位文案——`会话 <ts>` 占位没有稳定字面量可匹配。
        let is_first_message = session_messages
            .read()
            .get(&sid)
            .is_some_and(|msgs| msgs.is_empty());
        // 侧栏按钮新建的会话（created == None）落库时只有占位标题，首条
        // 消息后要把内存里已换成的截词标题补一次 `session.update_title`，
        // 否则刷新后回退占位。自建会话（created == Some）下方线程内已用
        // 派生标题 create，不需要重复 update。
        let needs_title_update = created.is_none() && is_first_message;

        // Daemon 模式：真运行。用户消息持久化（线程内，刚自建的会话在同
        // 一线程先 create 再 append 保证顺序）；assistant 事件全走订阅
        // 管线（见上方 effect），prompt 返回值（worker 原始 rpc 响应）
        // 忽略。run_target_sid 在 spawn 前落定，消费端据此写回本会话。
        // run_id 一并带给 daemon 记进 run ledger，刷新后列表据此组装
        // aborted/active（WP-C）。
        if let DataBackend::Daemon(d) = backend() {
            let sid_daemon = sid.clone();
            let text_daemon = text.clone();
            let d_prompt = d.clone();
            // 订阅事件不带会话归属：消费端以 run_target_sid 为写回目标，
            // 必须在 prompt 发出前落定
            run_target_sid.set(sid.clone());
            let run_id = format!("r-{}", now_ms());
            live_run_id.set(run_id.clone());
            // 5.6 计时起点：run 开始时落定 started_at，状态行的耗时段在飞
            // 期间随流式事件重渲染实时算，TurnEnd 时结算成总耗时。
            // 对位 dsh packages/client/runtime/.../assistant-timing.ts。
            let mut st_start = statusline();
            st_start.start_run(now_ms());
            statusline.set(st_start);
            eprintln!("[web] send sid={} len={}", sid, text.len());
            let (fail_tx, fail_rx) = tokio::sync::oneshot::channel::<()>();
            let run_id_prompt = run_id.clone();
            std::thread::spawn(move || {
                if let Some((id, _placeholder)) = created {
                    // 首条消息即标题来源：自建会话直接落库截词标题（走既有
                    // create_session，无新增 RPC）。
                    let _ = d.create_session(&id, &title_from_first_message(&text_daemon));
                }
                let _ = d.append_message(&sid_daemon, true, &text_daemon);
                if needs_title_update {
                    // 侧栏按钮新建的会话：占位标题已落库，首条消息后补一次
                    // 真 UPDATE 让刷新后的标题与内存一致。失败只降级不阻断
                    // 发送——标题回退占位好过消息发不出去。
                    if let Err(e) =
                        d.update_session_title(&sid_daemon, &title_from_first_message(&text_daemon))
                    {
                        eprintln!("[web] session title update failed: {e}");
                    }
                }
                if let Err(e) =
                    d_prompt.worker_prompt_run(&sid_daemon, &run_id_prompt, &text_daemon)
                {
                    // 确定性的失败点（daemon 掉线 / worker 拉起失败）：
                    // 事件路径不会有 TurnEnd 来复位。线程只发信号量，
                    // Signal 写回留在任务里（UnsyncStorage 非 Send）
                    eprintln!("[web] worker_prompt ERR: {e}");
                    let _ = fail_tx.send(());
                }
            });
            let sid_fail = sid.clone();
            spawn(async move {
                if fail_rx.await.is_ok() {
                    {
                        let mut map = space_sessions.write();
                        for list in map.values_mut() {
                            for s in list.iter_mut() {
                                if s.id == sid_fail {
                                    s.status = SessionStatus::Idle;
                                }
                            }
                        }
                    }
                    // 计时结算（5.6）：prompt 直接失败时事件路径不会有
                    // TurnEnd，不结算耗时会一直按「在飞」实时增长
                    let mut st = statusline();
                    st.finish_run(now_ms());
                    statusline.set(st);
                }
            });
        }

        let now = now_ms();
        let user_msg = ChatMessage {
            id: format!("{}-user-{}", sid, now),
            role: "user".into(),
            content: text.clone(),
            tool_calls: vec![],
            parts: vec![],
            timestamp: "刚刚".into(),
            ts_epoch_ms: now,
        };
        session_messages
            .write()
            .entry(sid.clone())
            .or_default()
            .push(user_msg);

        // 会话进入运行态
        let mut map = space_sessions.read().clone();
        for list in map.values_mut() {
            for s in list.iter_mut() {
                if s.id == sid {
                    s.status = SessionStatus::Active;
                    s.last_active = "刚刚".into();
                    s.last_active_epoch = now;
                    if is_first_message {
                        s.title = title_from_first_message(&text);
                    }
                }
            }
        }
        space_sessions.set(map);

        match backend() {
            DataBackend::Disconnected => {
                // 无 daemon = 无 agent 可跑：用户消息已上屏（本地即时反馈），
                // 但不产生任何 assistant 回复——旧的 mock 模拟流已删除，这里
                // 绝不编造回复。只把会话从上文刚置的 Active 复位回 Idle，
                // 否则 composer 会永久卡在「运行中」（停止钮亮、输入被门禁）。
                // 无 run 发起，statusline 计时也不启动（保持上一轮结算值）。
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
            DataBackend::Daemon(_) => {
                // 事件由订阅管线（见上方 effect）驱动，TurnEnd 收尾在订阅
                // 消费端完成；prompt 失败兜底已在上文挂接。
            }
        }
    };

    // ── 渲染 ────────────────────────────────────────────────────────────────

    let header_title = if view() == View::Stats {
        "数据统计".to_string()
    } else {
        active_title
    };

    // WP-C：侧栏会话列表喂入推断出的状态（在飞 Active 优先，其次 run 记录
    // 推断的缓存状态，最后会话自身状态）。Mock 无缓存，合并后与原样一致。
    let sidebar_sessions = merge_run_statuses(space_sessions.read().clone(), &run_status_cache());

    rsx! {
        div { class: "grid h-screen w-screen bg-base overflow-hidden relative select-none",
            style: "grid-template-columns: {grid_cols(sidebar_collapsed(), sidebar_width())};",
            onmousemove: on_root_mousemove,
            onmouseup: on_root_mouseup,
            Sidebar {
                spaces: spaces(),
                space_sessions: sidebar_sessions,
                active_id: active_session_id(),
                on_select: move |id: String| {
                    active_session_id.set(id);
                    view.set(View::Chat);
                },
                on_select_space: on_select_space,
                on_create: on_create_session,
                on_create_child: on_create_child,
                on_delete_session: on_delete_session,
                on_delete_space: on_delete_space,
                collapsed: sidebar_collapsed(),
                on_toggle: on_toggle_sidebar,
                on_expand: on_expand_sidebar,
                width: sidebar_width(),
                on_resize_start: on_resize_start,
                on_preset_start: move |x: i32| {
                    preset_dragging.set(true);
                    let _ = x;
                },
                on_open_search: move |_| show_quick_switcher.set(true),
                on_open_stats: move |_| view.set(View::Stats),
                on_open_settings: move |_| show_settings.set(true),
            }

            // 中栏：面包屑头 + 视图
            div { class: "min-w-0 flex flex-col bg-base overflow-hidden",
                div { class: "min-h-[44px] pl-7 pr-5 pt-3 pb-2 border-b border-b1 flex items-center gap-2 shrink-0",
                    if view() == View::Stats {
                        span { class: "text-[14px] leading-5 font-medium text-label", "数据统计" }
                    } else {
                        span { class: "text-[14px] leading-5 font-medium text-label", "{active_space.name}" }
                        if !active_space.branch.is_empty() {
                            span { class: "text-caption", "/" }
                            span { class: "text-[14px] leading-5 text-label-2 truncate max-w-[360px]", "{header_title}" }
                            span { class: "font-mono text-[11px] leading-4 px-2 py-0.5 rounded-full bg-chip-brand text-brand-300 border border-b1 shrink-0",
                                "{active_space.branch}"
                            }
                        }
                    }
                    div { class: "flex-1" }
                    if active_session_running(space_sessions, active_session_id) {
                        span { class: "flex items-center gap-1.5 text-[12px] leading-5 text-label-3",
                            span { class: "spinner-ring" }
                            "运行中"
                        }
                    }
                }
                match view() {
                    View::Stats => rsx! { StatsView {} },
                    View::Chat => rsx! {
                        Chat {
                            messages: current_messages,
                            statusline: statusline(),
                            is_streaming: active_session_running(space_sessions, active_session_id),
                            on_send: on_send,
                            on_model_change: on_model_change,
                            on_toggle_thinking: on_toggle_thinking,
                            on_abort: on_abort,
                            on_toggle_tasks: move |_| show_tasks.set(!show_tasks()),
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
            }

            // ⌘K 快速切换
            if show_quick_switcher() {
                Modal { width_class: "w-[560px]", top_aligned: true, on_close: move |_| show_quick_switcher.set(false),
                    div { class: "p-3 flex flex-col gap-2",
                        input {
                            class: "w-full h-11 rounded-xl bg-layer-1 border border-b2 px-4 text-[14px] leading-5 text-label outline-none transition-colors focus:border-brand placeholder:text-caption",
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
                                        "h-10 px-3 rounded-[10px] flex items-center justify-between cursor-pointer transition-colors bg-ihover"
                                    } else {
                                        "h-10 px-3 rounded-[10px] flex items-center justify-between cursor-pointer transition-colors hover:bg-ihover"
                                    };
                                    let dot = match session.status {
                                        SessionStatus::Active => "bg-brand",
                                        SessionStatus::Archived | SessionStatus::Aborted => "bg-danger/70",
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
                                                span { class: "text-[14px] leading-5 text-label truncate", "{session.title}" }
                                            }
                                            span { class: "font-mono text-[11px] leading-4 text-caption shrink-0", "{session.id}" }
                                        }
                                    }
                                }
                            }
                        }
                        div { class: "pt-2 border-t border-b1 flex items-center justify-between text-[12px] leading-4 text-caption",
                            span { "选择会话快速切换" }
                            div { class: "flex items-center gap-1.5",
                                kbd { "ESC" }
                                span { "退出" }
                            }
                        }
                    }
                }
            }

            // 设置弹窗
            if show_settings() {
                SettingsModal {
                    config: config.clone(),
                    on_update_config: on_update_config,
                    on_close: move |_| show_settings.set(false),
                }
            }
        }
    }
}

fn grid_cols(collapsed: bool, width: usize) -> String {
    if collapsed {
        "56px minmax(0,1fr)".to_string()
    } else {
        format!("{width}px minmax(0,1fr)")
    }
}

/// 当前会话是否在运行（会话级状态：composer 的停止钮/运行中标识/
/// 输入门禁都由此驱动，切会话自然切换）。
fn active_session_running(
    space_sessions: Signal<HashMap<String, Vec<Session>>>,
    active_session_id: Signal<String>,
) -> bool {
    let sid = active_session_id();
    !sid.is_empty()
        && space_sessions
            .read()
            .values()
            .flatten()
            .any(|s| s.id == sid && s.status == SessionStatus::Active)
}

/// 会话列表侧的有效状态：页面在飞（Active，由事件流实时写）优先于
/// run 记录推断出的缓存状态（WP-C），缓存缺失时保持会话自身状态。
fn effective_status(current: &SessionStatus, inferred: Option<&SessionStatus>) -> SessionStatus {
    match (current, inferred) {
        // 页面正在跑：以实时态为准（推断缓存是加载时的快照，会滞后）
        (SessionStatus::Active, _) => SessionStatus::Active,
        (_, Some(inferred)) => inferred.clone(),
        (other, None) => other.clone(),
    }
}

/// 把推断状态就地合并进侧栏用的 space → 会话列表映射。
fn merge_run_statuses(
    mut map: HashMap<String, Vec<Session>>,
    cache: &HashMap<String, SessionStatus>,
) -> HashMap<String, Vec<Session>> {
    for list in map.values_mut() {
        apply_run_statuses(list, cache);
    }
    map
}

/// 把推断状态就地合并进一个会话列表（⌘K 快速切换用）。
fn apply_run_statuses(list: &mut [Session], cache: &HashMap<String, SessionStatus>) {
    for s in list.iter_mut() {
        s.status = effective_status(&s.status, cache.get(&s.id));
    }
}
