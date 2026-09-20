//! 键盘焦点归属：窗口重新激活时，把键盘焦点送进**该拿它的那个 webview**。
//!
//! ## 现象
//!
//! 在界面上放好光标，切屏（Alt+Tab、切虚拟桌面、最小化到托盘）再回来，焦点就没了：
//! - 主界面（配置 / 设置页）的输入框失焦；
//! - **标签页里（内嵌页面里）的输入框也失焦**，打字不进去，得再点一下页面。
//!
//! ## 根因：wry 的 child webview 不接顶层窗口的 `WM_SETFOCUS`
//!
//! 这是 wry 的已知问题（`tauri-apps/wry#1754`，wry-0.55.1 仍未修）。机制是这样的：
//!
//! wry 把「宿主窗口拿到焦点」转成「WebView2 拿到焦点」的办法，是给**顶层窗口**挂一个
//! 子类过程，`WM_SETFOCUS` 时调 `ICoreWebView2Controller::MoveFocus(PROGRAMMATIC)`
//! （`webview2/mod.rs` 的 `parent_subclass_proc`）。但那个子类是
//! **`if !is_child { attach_parent_subclass(...) }`** —— 只给**非 child** 的 webview 装。
//!
//! 于是：
//! - 主 webview（非 child）本来靠它工作，**但** `impl Drop for InnerWebView` 里
//!   `dettach_parent_subclass(parent)` 是**无条件**执行的，`is_child == true` 的内嵌页面
//!   也会走到 —— **每销毁一个内嵌 webview（关标签页、LRU 回收），这条路由就被摘掉一次**，
//!   之后再也不会恢复（这个坑我自己踩到过，见提交历史里那版无效的修复）。
//! - 内嵌页面（`add_child` / `build_as_child` → `is_child == true`）**压根没装**。
//!   它的容器窗口过程确实会把焦点转给内部文档窗口（`default_window_proc` 里
//!   `SetFocus(GetWindow(hwnd, GW_CHILD))`），**可是 Alt+Tab 回来时 Windows 把
//!   `WM_SETFOCUS` 发给顶层窗口、而不是发给孩子容器** —— 所以那条路对「重新激活」这个
//!   场景完全不生效。
//!
//! 一句话：**顶层窗口拿到焦点的这件事，需要被转发进正确的 webview 里；wry 只转发了非 child 的那一个。**
//!
//! ## 做法
//!
//! 自己在顶层窗口上补一条子类过程，把 `WM_SETFOCUS` 转发给**此刻该拿键盘的那个 webview 容器**；
//! 激活后还按几个时间点重试 + 校验（系统与各 WebView2 各自异步地抢焦点，单发一次不稳）。
//!
//! 「该归谁」不去维护任何镜像状态，直接**问窗口本身**：只有当前显示着的那个内嵌页面容器是
//! `WS_VISIBLE`（wry 的 `set_visible` 会 `ShowWindow` 容器 HWND），其余都被 `hide_others`
//! 藏了。所以 —— 有可见的内嵌页面 → 归它；否则 → 归主 webview。这样不会出现
//! 「标志位和真实显隐不一致」那类错。
//!
//! 容器 HWND 的识别同理靠几何：主 webview 的容器贴客户区顶端（`resize_to_parent`），内嵌
//! 页面的容器被 `TOOLBAR_HEIGHT` 的留白顶下来，`top` 必然更大。
//!
//! ### 只做一层（HWND），坚决不碰 `MoveFocus` —— 这是拿日志换来的结论
//!
//! （当时往 `%APPDATA%\<id>\focus.log` 写诊断日志，破案后已连同日志一起删掉；
//! 想再查这类问题，见文末「诊断手段」。）
//!
//! 修这个 bug 一共走了四版，前三版的教训都值得记：
//!
//! 1. **加 `MoveFocus(PROGRAMMATIC)` 补「元素层」→ 输入框高频闪烁。**
//!    `MoveFocus` 会让 WebView2 先把宿主**顶层窗口**重新置为焦点 → tao 再报一次
//!    `Focused(true)` → 又走回本模块 → 又 `MoveFocus` —— **自我循环**，实测 ~95 次/秒
//!    （日志里相邻两行只差 11ms，而重试表是 `[0,40,120,300,700]`，这个间隔本身
//!    就说明是每轮都在新起 claim）。而且循环里容器**一直是有焦点的**，「已就位就不动」
//!    识别不出它。
//! 2. **按容器冷却 600ms 止循环 → 焦点又丢了。** 冷却把重试一起掐死，元素级永远只补一发。
//! 3. **改成「本轮每个重试点都补」→ 焦点被送去页面左上角。** 探测页面才发现真相（见下）。
//!
//! **`MoveFocus(PROGRAMMATIC)` 的真实语义**（MS 文档 + 实测）：焦点落到**先前聚焦的那个
//! 元素**，**没有就落到文档第一个元素**。而「先前聚焦的元素」这个状态一旦被它自己破坏
//! （送到 BODY 或第一个 `<a>`），之后每次补都回到那个错误元素 —— 也就是说它不是恢复，
//! 而是**在破坏**。当时的页面探测日志（现已移除）：
//!
//! ```text
//! 起始/页面: hasFocus=false active=INPUT   ← 切屏根本没动 activeElement！
//! 结束/页面: hasFocus=true  active=BODY    ← 是 MoveFocus 把它改掉的
//! 结束/页面: hasFocus=true  active=A       ← 「焦点跑到左上角」的元凶
//! ```
//!
//! 所以正确的模型是：**HWND 层归 Rust，元素层归页面**。
//!
//! - **HWND 层**（本模块）：`SetFocus(该拿键盘的容器)`。这一层之外不要再动 WebView2 的
//!   焦点 API —— `MoveFocus` 的副作用（自我循环、把 activeElement 冲成第一个元素）都盖过收益。
//! - **元素层**（页面里）：WebView2/wry 在 webview 重新拿到焦点时会做一次「聚焦第一个
//!   可聚焦元素」，落在 DOM 上就是「左上角那个 logo/图标按钮」。所以页面侧必须自己记住
//!   「用户最后编辑的字段」并把它抢回来 —— 见 `embed.rs::INIT_SCRIPT`（内嵌页）与前端
//!   `lib/focus.ts`（主界面）。恢复**只能**用 `element.focus()`，绝不能用 `MoveFocus`。
//!   激活链末端由 [`restore_dom_focus`] 再补一次。
//!
//! `embed.rs` 建 webview 时必须 `.focused(false)`：wry 的默认值 `true` 会在**创建时**
//! 就调一次 `controller.MoveFocus(PROGRAMMATIC)`（wry `webview2/mod.rs:546`），
//! 新页面的 activeElement 会被定到第一个元素。
//!
//! ## 诊断手段（下次再查这类问题就看这里）
//!
//! 焦点问题的排查**必须先有证据**，否则一定猜错（上述三版错误修法都是猜出来的）：
//!
//! - **HWND 层**：`GetFocus()` 沿 `GetParent` 往上看落在哪个 `WRY_WEBVIEW` 容器里；
//!   容器用类名 + 几何区分（主 webview 的 `top` 最小）。
//! - **DOM 层**（关键）：Rust 看不到页面里的 `activeElement`，只能 `eval` 去问页面 ——
//!   `document.hasFocus()` + `document.activeElement.tagName/.id/.className`。
//!   把它按「每个 webview 一行」写进日志，一次激活记「起 / 止」两批，就能看出
//!   是谁在什么时候动了元素焦点。注意 tauri 的 `eval_with_callback` 在 Windows 上
//!   **会吞掉 JS 异常**（脚本要自己 try/catch），而 `eval` 是**异步**的、只能用于诊断。
//! - 落盘日志位置建议放在**程序目录**（见 `store.rs::data_dir`），不要写 %APPDATA%；
//!   记得带体积上限（超限截尾），否则一次焦点风暴能写出上百 KB。

use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};

/// wry 给每个 webview 建的容器窗口类名（wry-0.55.1 `webview2/mod.rs::create_container_hwnd`）。
///
/// 主 webview 和每个内嵌页面都是一个这样的 `WS_CHILD` 窗口，父窗口都是主窗口，彼此是兄弟。
#[cfg(windows)]
const WRY_CONTAINER_CLASS: &str = "WRY_WEBVIEW";

/// 主 webview 的容器 HWND。全进程唯一定值，首次解析成功后缓存。
static MAIN_CONTAINER: AtomicIsize = AtomicIsize::new(0);

/// 抢焦点轮次：新一轮激活会让上一轮残留的定时重试作废，避免反复切换叠加出一堆后台线程。
static CLAIM_ROUND: AtomicUsize = AtomicUsize::new(0);

/// 激活后重试的时间点（毫秒，相对于激活时刻）。
///
/// 为什么不止一发：系统把焦点给了谁、以及各容器自己的窗口过程怎么接力，都是异步发生的，
/// 谁在后面谁赢。**整张表都走完**（不在第一发就位时提前收工）：wry 挂在顶层窗口上的
/// 子类过程会在激活后**异步**再抢一次焦点，早退正好漏掉它。每一发都是幂等的
/// （见 [`focus_container`]：已经在目标容器里就什么都不做），所以顺利时没有多余动作。
const CLAIM_SCHEDULE_MS: [u64; 5] = [0, 40, 120, 300, 700];

/// 把键盘焦点要回**主 webview**（不管内嵌页面是不是显示着）。
///
/// 给 `focus_webview` 命令用：前端在「刚把内嵌页面藏起来、该由主界面接手键盘」时明确要求。
/// 激活时的自动归属不用这个 —— 那个按窗口真实显隐判断（见 [`claim_deferred`]）。
#[cfg(windows)]
pub fn claim_main(app: &tauri::AppHandle) {
    let Some(window) = main_window_hwnd(app) else {
        return;
    };
    let Some(container) = main_container(window) else {
        return;
    };
    // 这里只管 HWND 层。前端 `lib/focus.ts` 在「主界面需要补选具体元素」时自己处理，
    // 与本模块无关。
    unsafe { focus_container(container) }
}

#[cfg(not(windows))]
pub fn claim_main(_app: &tauri::AppHandle) {}

/// 把键盘交给「此刻该拿它的那个 webview」（可见内嵌页优先，否则主界面）。
///
/// 给**用户主动打开 / 切换标签页**用：与激活链同一套判据、同一层实现（`SetFocus(容器)`）。
///
/// 为什么不用 `Webview::set_focus()`：那是在 `MoveFocus(PROGRAMMATIC)`，会把页面里的
/// `activeElement` 冲成 BODY 或文档第一个元素 —— 表现就是「焦点跑到左上角」
/// （详见模块头「只做一层」）。`SetFocus(容器)` 只把键盘送进 WebView2 的窗口，
/// 页面里先前聚焦的元素由浏览器 + 内嵌页注入的兜底脚本一起恢复。
///
/// 立即返回：`SetFocus` 必须在窗口所属线程上调用，这里派回主线程执行。
pub fn claim_visible(app: &tauri::AppHandle) {
    #[cfg(windows)]
    {
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            claim_owner(&handle);
        });
    }
    #[cfg(not(windows))]
    let _ = app;
}

/// 激活后按 [`CLAIM_SCHEDULE_MS`] 走完整张时间表，「把焦点交给该拿它的 webview」。
///
/// 立即返回：定时点在后台线程里等，到点了再 `run_on_main_thread` 回主线程执行
/// （`SetFocus` / `GetFocus` 必须在窗口所属线程上调用）。
///
/// **刻意不在「第一发就位」时收工**：wry 自己挂在顶层窗口上的子类过程
/// （`webview2/mod.rs` 的 `parent_subclass_proc`，`WM_SETFOCUS → controller.MoveFocus(PROGRAMMATIC)`）
/// 会在激活后**异步**地再抢一次焦点，且它去抢的是**它自己那个 webview** ——
/// 早退就正好漏掉它。所以这里把整张表都走完，每次发现焦点不在位就再要回来一次
/// （`focus_container` 里「已就位就不动」，所以顺利时不会有多余的 `SetFocus`）。
pub fn claim_deferred(app: &tauri::AppHandle) {
    let round = CLAIM_ROUND.fetch_add(1, Ordering::SeqCst) + 1;
    let handle = app.clone();

    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        for target in CLAIM_SCHEDULE_MS {
            let elapsed = started.elapsed().as_millis() as u64;
            if let Some(wait) = target.checked_sub(elapsed) {
                if wait > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(wait));
                }
            }

            // 期间又发生了一次激活 → 让位给新一轮
            if CLAIM_ROUND.load(Ordering::SeqCst) != round {
                return;
            }

            if handle.run_on_main_thread({
                let worker = handle.clone();
                move || claim_owner(&worker)
            }).is_err() {
                return;
            }
        }

        // 最后一刀：主动让页面把「用户最后编辑的字段」再要回来一次。
        // 不赌 `window.focus` 事件的时序 —— 浏览器/WebView2 的「聚焦第一个元素」行为
        // 可能发生在我们校验之后，这一步是兜底里的兜底。
        restore_dom_focus(&handle);
    });
}

/// 让每个 webview **页面自己**把「用户最后编辑的字段」再要回来一次。
///
/// Rust 侧只能管到 HWND 层（键盘进哪个 webview 的窗口）；页面里 `activeElement` 落在
/// 哪个元素上只有页面自己知道（见 `embed.rs::INIT_SCRIPT` 与前端 `lib/focus.ts` ——
/// 它们各自往 `window.__WEBLAUNCH_RESTORE_FOCUS__` 上挂了一个钩子）。
///
/// 为什么不赌 `window.focus` 事件的时序：浏览器/WebView2 的「聚焦第一个可聚焦元素」
/// 是**晚一步**发生的，可能落在页面自己所有定时恢复点之后；由宿主在激活链走完时
/// 再拉一把最稳。
#[cfg(windows)]
fn restore_dom_focus(app: &tauri::AppHandle) {
    use tauri::Manager;

    // 钩子会「先补一次焦点、再回报状态」；没有钩子的（例如设置页以外的视图）就什么都不做。
    const JS: &str = r#"
(function () {
  try {
    if (typeof window.__WEBLAUNCH_RESTORE_FOCUS__ === 'function') {
      window.__WEBLAUNCH_RESTORE_FOCUS__();
    }
  } catch (e) { /* 页面自己的脚本出错不该影响宿主 */ }
})()
"#;

    for (_label, webview) in app.webviews() {
        let _ = webview.eval(JS);
    }
}

#[cfg(not(windows))]
fn restore_dom_focus(_app: &tauri::AppHandle) {}

/// 在主窗口上挂子类过程：顶层窗口拿到 `WM_SETFOCUS` 时，把焦点转发进该拿它的 webview。
///
/// **这就是补 wry 缺的那一环**：Windows 在重新激活时把 `WM_SETFOCUS` 发给顶层窗口，
/// 而 wry 只给非 child 的 webview 装了这个转发（见模块头注释）。
#[cfg(windows)]
pub fn install(app: &tauri::AppHandle) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::WM_SETFOCUS;

    /// 子类过程 id：不与 wry / tao / frameless 撞（"WLFK"）
    const SUBCLASS_ID: usize = 0x57_4C_46_4B;

    unsafe extern "system" fn forward_focus(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _uid: usize,
        _data: usize,
    ) -> LRESULT {
        if msg == WM_SETFOCUS {
            // 这里只有 HWND、拿不到 AppHandle，就只做这一件事：SetFocus 到容器，
            // 容器自己的窗口过程会把焦点再交给它内部的文档窗口。
            // 不碰 MoveFocus —— 见模块头「只做一层」。
            if let Some(owner) = keyboard_owner(hwnd) {
                if !focus_inside(owner) {
                    SetFocus(owner);
                }
            }
        }
        unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
    }

    let Some(hwnd) = main_window_hwnd(app) else {
        return;
    };

    unsafe {
        SetWindowSubclass(hwnd, Some(forward_focus), SUBCLASS_ID, 0);
    }
}

#[cfg(not(windows))]
pub fn install(_app: &tauri::AppHandle) {}

/* ---------------------------------- Windows 实现 ---------------------------------- */

#[cfg(windows)]
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};

#[cfg(windows)]
fn main_window_hwnd(app: &tauri::AppHandle) -> Option<HWND> {
    use tauri::Manager;

    let window = app.get_window("main")?;
    let handle = window.hwnd().ok()?;
    Some(handle.0 as HWND)
}

/// 把焦点交给「此刻该拿它的 webview」（显示着的内嵌页面，没有就主界面）。
#[cfg(windows)]
fn claim_owner(app: &tauri::AppHandle) {
    let Some(window) = main_window_hwnd(app) else {
        return;
    };
    let (main, embed) = unsafe { classify_containers(window) };

    // 显示着的内嵌页面优先；否则主界面。判定完全来自窗口真实显隐，没有镜像状态。
    if let Some(owner) = embed.or(main) {
        unsafe { focus_container(owner) };
    }
}

/// 把键盘焦点搬到某个容器上（幂等：已经在里面就什么都不做）。
///
/// **只有这一层**。键盘回到 WebView2 的窗口后，页面里先前聚焦的元素会自己把光标接回来
/// （切屏只动 `document.hasFocus()`，从不清掉 `document.activeElement`）。
/// 千万别再「帮一把」`MoveFocus(PROGRAMMATIC)`：它会把 `activeElement` 冲掉、送到
/// BODY 或文档第一个元素，也就是「焦点跑到左上角」的元凶（见模块头）。
///
/// 刻意**不校验窗口在不在前台**：激活事件与 `GetForegroundWindow()` 的更新并不同步
/// （窗口确实在被激活、只是系统还没标成前台），而 `SetFocus` 到本线程的窗口
/// 不会抢别的应用前台，无害。
#[cfg(windows)]
unsafe fn focus_container(container: HWND) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;

    unsafe {
        if !focus_inside(container) {
            SetFocus(container);
        }
    }
}

/// 此刻键盘该归哪个 webview 的容器。
///
/// 判据全部来自窗口自身，不维护镜像状态：
/// - 有**显示着**的内嵌页面（`WS_VISIBLE` 且被工具栏留白顶下来）→ 归它；
/// - 否则归主 webview（贴客户区顶端、面积最大的那个容器）。
#[cfg(windows)]
fn keyboard_owner(window: HWND) -> Option<HWND> {
    use windows_sys::Win32::UI::WindowsAndMessaging::IsWindowVisible;

    unsafe {
        let (main, embed) = classify_containers(window);
        if let Some(embed) = embed {
            // 再确认一次真的可见：容器被隐藏时不该把键盘交给它
            if IsWindowVisible(embed) != 0 {
                return Some(embed);
            }
        }
        main
    }
}

/// 主 webview 的容器 HWND（带缓存，窗口销毁后自动失效重解析）。
#[cfg(windows)]
fn main_container(window: HWND) -> Option<HWND> {
    use windows_sys::Win32::UI::WindowsAndMessaging::IsWindow;

    let cached = MAIN_CONTAINER.load(Ordering::Relaxed);
    if cached != 0 {
        let hwnd = cached as HWND;
        if unsafe { IsWindow(hwnd) } != 0 {
            return Some(hwnd);
        }
        MAIN_CONTAINER.store(0, Ordering::Relaxed);
    }

    let (main, _) = unsafe { classify_containers(window) };
    let found = main?;
    MAIN_CONTAINER.store(found as isize, Ordering::Relaxed);
    Some(found)
}

/// 把主窗口的 `WRY_WEBVIEW` 子窗口分成「主 webview 容器」与「显示着的内嵌页面容器」。
///
/// 不能靠「`GW_CHILD` 的第一个」：`CreateWindowExW` 把新窗口插到兄弟链**顶端**，wry 建完容器
/// 还会 `SetWindowPos(HWND_TOP)` 再抬一次 —— 第一个反而是**最新**建的那个（某个内嵌页面）。
///
/// 几何判据：主 webview 的容器贴客户区顶端，内嵌页面的容器由 `TOOLBAR_HEIGHT` 的留白顶下来，
/// 所以 **`top` 最小的那个是主 webview**，`top` 更大的那些是内嵌页面。
/// 用屏幕坐标的绝对 `top` 比较（两者左边界相同），窗口被拖到屏幕外也不会错。
#[cfg(windows)]
unsafe fn classify_containers(window: HWND) -> (Option<HWND>, Option<HWND>) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindow, GetWindowRect, IsWindowVisible, GW_CHILD, GW_HWNDNEXT,
    };

    struct Candidate {
        hwnd: HWND,
        top: i32,
        area: i64,
        visible: bool,
    }

    let mut all: Vec<Candidate> = Vec::new();
    let mut child = GetWindow(window, GW_CHILD);

    while !child.is_null() {
        if class_of(child).as_deref() == Some(WRY_CONTAINER_CLASS) {
            let mut rect: RECT = std::mem::zeroed();
            if GetWindowRect(child, &mut rect) != 0 {
                all.push(Candidate {
                    hwnd: child,
                    top: rect.top,
                    area: (rect.right - rect.left) as i64 * (rect.bottom - rect.top) as i64,
                    visible: IsWindowVisible(child) != 0,
                });
            }
        }
        child = GetWindow(child, GW_HWNDNEXT);
    }

    // 主 webview = top 最小者；并列（新建的容器在被 apply_bounds 排布前是 1×1 占位，
    // 位置也在左上角）时取面积最大的，一定是铺满客户区的那个。
    let main_index = all
        .iter()
        .enumerate()
        .min_by_key(|(_, c)| (c.top, -c.area))
        .map(|(i, _)| i);

    let Some(main_index) = main_index else {
        return (None, None);
    };
    let main = all[main_index].hwnd;

    // 内嵌页面 = 其余里第一个显示着的。面积 > 1 是为了排掉「刚 add_child 出来、还没被
    // apply_bounds 排布过」的 1×1 占位窗口（那时窗口不可见，本来也轮不到它）。
    let embed = all
        .iter()
        .enumerate()
        .filter(|(i, c)| *i != main_index && c.visible && c.area > 1)
        .map(|(_, c)| c.hwnd)
        .next();

    (Some(main), embed)
}

/// 焦点（含其祖先链）是否落在 `ancestor` 之内。
///
/// 焦点实际在容器的**子**窗口上（WebView2 的文档窗口），所以不能直接比 HWND，
/// 得顺着 `GetParent` 往上走。
#[cfg(windows)]
fn focus_inside(ancestor: HWND) -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetParent;

    unsafe {
        let mut current = GetFocus();
        while !current.is_null() {
            if current == ancestor {
                return true;
            }
            current = GetParent(current);
        }
        false
    }
}

#[cfg(windows)]
fn class_of(hwnd: HWND) -> Option<String> {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetClassNameW;

    let mut buffer = [0u16; 256];
    let len = unsafe { GetClassNameW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    if len <= 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&buffer[..len as usize]))
}
