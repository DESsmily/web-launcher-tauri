mod appmenu;
mod embed;
mod frameless;
mod service;
mod store;
mod winfocus;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, RunEvent, State, WindowEvent,
};

use embed::Insets;
use service::ServiceManager;
use store::{Store, StoreState, CLOSE_ACTION_EXIT};

/* ---------------------------------- 配置存储 ---------------------------------- */

#[tauri::command]
fn load_store(state: State<'_, StoreState>) -> Store {
    state.read()
}

#[tauri::command]
fn save_store(state: State<'_, StoreState>, store: Store) -> Result<(), String> {
    state.write(store)
}

#[tauri::command]
fn set_close_action(state: State<'_, StoreState>, action: String) -> Result<(), String> {
    let mut snapshot = state.read();
    snapshot.settings.close_action = action;
    state.write(snapshot)
}

#[tauri::command]
fn app_info(app: AppHandle) -> serde_json::Value {
    serde_json::json!({
        "name": "Web 启动器",
        "version": app.package_info().version.to_string(),
        "tauri": tauri::VERSION,
    })
}

/* ---------------------------------- 服务进程 ---------------------------------- */

#[tauri::command]
fn check_port(port: u16) -> bool {
    service::check_port(port)
}

#[tauri::command]
fn start_service(
    app: AppHandle,
    mgr: State<'_, ServiceManager>,
    id: String,
    command: String,
    cwd: Option<String>,
) -> Result<u32, String> {
    service::start(&app, &mgr, &id, &command, cwd.as_deref().unwrap_or(""))
}

#[tauri::command]
fn stop_service(mgr: State<'_, ServiceManager>, id: String) {
    service::stop(&mgr, &id);
}

#[tauri::command]
fn stop_all_services(mgr: State<'_, ServiceManager>) {
    service::stop_all(&mgr);
}

#[tauri::command]
fn running_services(mgr: State<'_, ServiceManager>) -> Vec<String> {
    mgr.list()
}

/// 在指定目录下打开一个终端窗口。
///
/// `cwd` 由前端决定（标签页对应配置的工作目录；非本地启动 / 未填目录时传 `None`），
/// Rust 侧只负责校验目录是否存在并拉起 `cmd.exe`，见 `service::open_terminal`。
#[tauri::command]
fn open_terminal(cwd: Option<String>) -> Result<(), String> {
    service::open_terminal(cwd.as_deref())
}

/* --------------------------------- 内嵌 WebView -------------------------------- */

#[tauri::command]
async fn open_embed(app: AppHandle, tab: String, url: String, insets: Insets) -> Result<(), String> {
    embed::open(&app, &tab, &url, insets)
}

#[tauri::command]
async fn set_embed_insets(app: AppHandle, insets: Insets) -> Result<(), String> {
    embed::set_insets(&app, insets);
    Ok(())
}

#[tauri::command]
async fn show_embed(app: AppHandle, tab: String) -> Result<(), String> {
    embed::show(&app, &tab)
}

#[tauri::command]
async fn hide_embeds(app: AppHandle) -> Result<(), String> {
    embed::hide_all(&app)
}

#[tauri::command]
async fn reload_embed(app: AppHandle, tab: String) -> Result<(), String> {
    embed::reload(&app, &tab)
}

#[tauri::command]
async fn close_embed(app: AppHandle, tab: String) -> Result<(), String> {
    embed::close(&app, &tab)
}

/* ---------------------------------- 窗口控制 --------------------------------- */

/// 窗口从托盘还原后通知前端：按当前视图重新同步一次内嵌页面显隐。
///
/// 隐藏到托盘已经不再动内嵌页面（见 `focus_main` 的说明），所以这里只作兜底 ——
/// 万一隐藏期间前端切过视图，显隐需要按新视图纠正。
pub const RESTORED_EVENT: &str = "window://restored";

/// 窗口重新获得焦点事件（Rust -> 主界面）。
///
/// 多实例 WebView2 下，窗口重新激活时系统可能把键盘焦点还给某个内嵌子 webview
/// （wry 给每个 webview 的 HWND 都挂了「收到 `WM_SETFOCUS` 就 `MoveFocus`」的子类过程），
/// 主界面正在编辑的输入框会跟着丢焦点。这个事件让前端把焦点补回来，
/// 见 `src/lib/focus.ts`。
pub const FOCUSED_EVENT: &str = "window://focused";

/// 窗口控制按钮（顶栏右侧的最小化 / 最大化 / 关闭）走这些命令。
fn main_window(app: &AppHandle) -> Result<tauri::Window, String> {
    app.get_window("main").ok_or_else(|| "主窗口不存在".to_string())
}

/// 把键盘焦点要回**主界面**（主 webview）。
///
/// 前端只在「不该由内嵌页面拿键盘」时才调它（例如刚把内嵌页面藏起来）。窗口激活时的自动
/// 归属不走这里 —— 那个按窗口真实显隐判断该归谁，见 [`winfocus::claim_deferred`]。
///
/// 为什么必须由 Rust 来做：DOM 层拿不到「键盘焦点在哪个 webview 上」这件事，
/// 而 `element.focus()` 在元素已经是 `document.activeElement` 时是空操作 ——
/// 恰恰最常见的情况就是「DOM 还记得那个输入框，但系统把键盘焦点给了内嵌页面」。
/// 具体怎么抢（以及为什么不能用 `Webview::set_focus()`）见 `winfocus.rs`。
#[tauri::command]
fn focus_webview(app: AppHandle) -> Result<(), String> {
    // 抢不到不算错误：窗口可能已经不在前台，或者 webview 还没建好。
    winfocus::claim_main(&app);
    Ok(())
}

/// 隐藏到托盘：**只隐藏主窗口**。
///
/// 上一版这里还多调了一次 `embed::hide_all()`，而那正是「托盘还原不了」的来源 ——
/// 内嵌页面并不是独立窗口，它是主窗口的 `WS_CHILD` 子窗口（wry 用
/// `CreateWindowExW(..., WS_CHILD, parent=主窗口, ...)` 建的），父窗口 `SW_HIDE`
/// 之后子窗口本来就不再合成，不需要、也不该再逐个藏一遍。
///
/// 逐个藏会凭空造出**两个显隐所有者**：主窗口由 Rust 恢复、内嵌页面得等前端收到
/// `RESTORED_EVENT` 再 `show_embed()` 才回来。这条 `invoke` 往返里只要断一环，窗口
/// 回来了内容区也是空的，而且从外面完全看不出断在哪一环。只隐藏主窗口之后，显隐只有
/// 一个所有者，「还原」退化成一次无条件的 `ShowWindow`。
///
/// （切到配置 / 设置 / 编辑视图时仍然走 `embed::hide_all()` —— 那里内嵌页面确实该藏。）
fn hide_to_tray(app: &AppHandle) {
    if let Ok(window) = main_window(app) {
        let _ = window.hide();
    }
}

#[tauri::command]
fn window_minimize(app: AppHandle) -> Result<(), String> {
    main_window(&app)?.minimize().map_err(|e| e.to_string())
}

#[tauri::command]
fn window_toggle_maximize(app: AppHandle) -> Result<(), String> {
    let window = main_window(&app)?;
    let maximized = window.is_maximized().map_err(|e| e.to_string())?;
    if maximized {
        window.unmaximize().map_err(|e| e.to_string())
    } else {
        window.maximize().map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn window_is_maximized(app: AppHandle) -> Result<bool, String> {
    main_window(&app)?.is_maximized().map_err(|e| e.to_string())
}

/// 顶栏关闭按钮：按设置里的「关闭操作」当场决定隐藏还是退出。
///
/// **这里必须同步判定，不能再借 `Window::close()` 绕一圈 CloseRequested。**
/// 原因在框架实现里写得很直白（tauri-runtime-wry）：
/// - `hide()` / `show()` / `minimize()` 走 `send_user_message`，在主线程上会
///   **内联同步执行**（`if current_thread().id() == context.main_thread_id`）；
/// - `close()` 是唯一例外，注释明写 "close cannot use the `send_user_message`
///   function"，只能 `proxy.send_event` → `PostMessageW` **异步投递**到事件循环。
///
/// 于是「最小化到托盘」这条路径就多了一次跨线程投递：一旦投递没能及时/成功送达
/// 事件循环（打开了标签页时消息量本来就大，事件循环还常被内嵌 WebView2 的回调
/// 占住），命令返回 Err，而前端把错误吞掉了 → 表现为「点关闭没反应」。改成直接
/// `hide()` 后这条路径和最小化 / 最大化完全一致，不再有可失败的中转。
///
/// 关闭策略仍然是「一处判定」：托盘在 `hide_to_tray`，退出交给 `CloseRequested`
/// 兜底（Alt+F4 / 任务栏关闭也走同一个 `on_window_event` 分支）。
#[tauri::command]
fn window_close(app: AppHandle) -> Result<(), String> {
    let close_action = app.state::<StoreState>().settings().close_action;

    if close_action == CLOSE_ACTION_EXIT {
        // 退出：走 Window::close() → CloseRequested，由 on_window_event 统一收尾
        return main_window(&app)?.close().map_err(|e| e.to_string());
    }

    hide_to_tray(&app);
    Ok(())
}

#[tauri::command]
async fn show_tab_menu(app: AppHandle, tab: String, can_restart: bool) -> Result<(), String> {
    appmenu::popup_tab_menu(app, tab, can_restart).await
}

/* ---------------------------------- 系统能力 ---------------------------------- */

/// 交给系统默认浏览器打开（Windows 走 `ShellExecuteW`）。
///
/// 不要再用 `cmd /C start "" <url>`：cmd.exe 会把整条命令行**重新解析一遍**，URL 里
/// 的 `&`、`^` 会被当成命令分隔符 / 转义符 —— `https://host/?a=1&b=2` 传到浏览器手里
/// 只剩 `https://host/?a=1`，运气差还能拼出第二条命令。opener 插件底层是
/// `ShellExecuteW`，参数原样交给 shell，既能正确处理任意 URL，也不经过 shell 解析。
pub(crate) fn open_in_system_browser(url: &str) -> Result<(), String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("地址为空".to_string());
    }
    tauri_plugin_opener::open_url(url, None::<&str>).map_err(|e| format!("调用系统浏览器失败：{e}"))
}

#[tauri::command]
fn open_external(url: String) -> Result<(), String> {
    open_in_system_browser(&url)
}

/* ------------------------------------ 托盘 ----------------------------------- */

/// 从托盘还原主窗口并置前。
///
/// 顺序是**先 Win32、后 tao**，两个原因：
///
/// 1. tao 的 `show()` 只做 `WindowFlags::VISIBLE` 的 diff，而 `apply_diff()` 第一行
///    就是 `if diff == empty { return }`：只要 tao 的内部标记已经是「可见」而真实
///    HWND 还停在 `SW_HIDE`，`show()` 会**静默退化成空操作**。先无条件 Win32 显示、
///    再让 tao 追状态，两个方向都安全 —— tao 只会把标记修正到与真实一致，之后的
///    `hide()` 也照旧生效。
/// 2. `SW_SHOW` 对**被最小化**的窗口不生效（只在当前位置显示），所以这里按
///    `IsIconic` 分流：最小化过走 `SW_RESTORE`，只是被 `SW_HIDE` 藏起来的才用
///    `SW_SHOW` —— 反过来一律用 `SW_RESTORE` 会把最大化过的窗口降回普通大小。
///
/// 另外这里**不再调 `window.set_focus()`**：tao 的 `set_focus` 在抢不到前台时会走
/// `force_window_active`，里面用 `SendInput` 合成一次 Alt 按下 / 抬起去骗前台权限。
/// 那是给「窗口创建」场景写的 hack（框架注释自己也这么写），不该出现在托盘还原这种
/// 随时会被触发的路径上。
///
/// 关键：**不要通过 `app.get_webview_window("main")` 拿窗口**。存在子 webview 且
/// 主窗口被 `SW_HIDE` 隐藏到托盘后，Tauri 运行时可能暂时丢失「main」这个
/// WebviewWindow 记录，导致该查找返回 `None` 并让还原逻辑提前 `return`。托盘
/// 还原只需要操作顶层 HWND，用 `app.get_window("main")` 拿到的 `tauri::Window`
/// 不受影响，事件也不依赖 WebviewWindow 包装。
fn focus_main(app: &AppHandle) {
    let Ok(window) = main_window(app) else {
        return;
    };

    #[cfg(windows)]
    if let Ok(handle) = window.hwnd() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            IsIconic, SetForegroundWindow, SetWindowPos, ShowWindow, HWND_TOP, SWP_NOMOVE,
            SWP_NOSIZE, SWP_SHOWWINDOW, SW_RESTORE, SW_SHOW,
        };

        let hwnd = handle.0 as *mut core::ffi::c_void;
        unsafe {
            ShowWindow(hwnd, if IsIconic(hwnd) != 0 { SW_RESTORE } else { SW_SHOW });
            // 抬到 Z 序顶部；显式带上 SWP_SHOWWINDOW 是为了绕开
            // 「系统认为窗口已可见」时的空操作
            SetWindowPos(hwnd, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW);
            // 无边框窗口没有标题栏，Windows 的前台锁定会挡掉 tao 内部那次「请求激活」，
            // 这里在窗口已经可见的前提下直接抢一次
            SetForegroundWindow(hwnd);
        }
    }

    // Win32 已经把窗口显示出来了；这两步只是把 tao 的内部标记追上真实状态，
    // 保证后续的 hide() 仍然有效
    let _ = window.show();
    let _ = window.unminimize();

    // 按当前视图重新同步一次内嵌页面显隐。内嵌页面现在跟着主窗口一起回来了，
    // 这一步属于兜底：万一隐藏期间前端切过视图，显隐要按新视图纠正。
    // emit 失败（例如运行时暂时丢了 main webview 记录）也无伤大雅，因为子
    // webview 是 WS_CHILD，父窗口显示后它们会随 WS_VISIBLE 状态自动恢复。
    let _ = app.emit_to("main", RESTORED_EVENT, ());
}

// 这里原来有一份往 `%APPDATA%\<id>\focus.log` 写的诊断日志（`log_window_state` 记窗口
// 真实可见/最小化/前台状态，`focus_log` 带体积上限地追加），是排查「切屏回来焦点丢失 /
// 焦点跳到左上角」时加的。问题修好后已整体删除：正常运行时不该往系统盘写这些。
// 将来还要查这类问题，先看 `winfocus.rs` 模块头的「诊断手段」一节，那里记了怎么重建。

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let show_item = MenuItem::with_id(app, "show", "显示主界面", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, "quit", "退出程序", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_item, &separator, &quit_item])?;

    let mut builder = TrayIconBuilder::with_id("web-launch-tray")
        .menu(&menu)
        // 左键 = 还原窗口，右键 = 弹出菜单（默认值 true 会让左键去弹菜单，
        // 把下面的 Click 处理器彻底架空，表现为「点托盘没反应」）
        .show_menu_on_left_click(false)
        .tooltip("Web 启动器");

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    builder
        .on_menu_event(|app, event| {
            match event.id().as_ref() {
                "show" => focus_main(app),
                "quit" => {
                    let mgr = app.state::<ServiceManager>();
                    service::stop_all(&mgr);
                    app.exit(0);
                }
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                focus_main(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

/* ------------------------------------ 入口 ----------------------------------- */

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // WebView2 的浏览器配置 / 缓存默认落在 `%LOCALAPPDATA%\<exe>.WebView2`（C 盘用户目录）。
    // 用官方支持的 `WEBVIEW2_USER_DATA_FOLDER` 把它挪到**程序目录**下的 `data/webview2`，
    // 与 `store.rs::data_dir` 保持同一套「数据跟着程序走、不写系统盘」的约定。
    //
    // 必须在**任何 webview 创建之前**设置（`tauri::Builder` 一 build 就会建主窗口的
    // webview）；写不进去（例如装在 Program Files 且无管理员权限）就不设，
    // 退回 WebView2 的默认位置。
    //
    // 注意：只设这个环境变量、**不要**给个别 webview 单独指定 `data_directory` ——
    // 那样会让它们用上不同的用户数据目录，等于互不相识（cookie / 登录态都不共享）。
    #[cfg(windows)]
    if let Some(dir) = store::install_data_dir() {
        let webview_dir = dir.join("webview2");
        if std::fs::create_dir_all(&webview_dir).is_ok() {
            std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", &webview_dir);
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(ServiceManager::default())
        .manage(embed::EmbedState::default())
        .setup(|app| {
            let handle = app.handle().clone();
            app.manage(StoreState::new(&handle));

            // 无边框窗口补丁：最大化限制到工作区，避免盖住任务栏
            frameless::install(&handle);

            // 键盘焦点路由：顶层窗口拿到焦点时把它转进主 webview。
            // wry 本来也挂了一条，但它销毁内嵌 webview 时会无条件摘掉（见 winfocus），
            // 所以这里自己补一条，不依赖它。
            winfocus::install(&handle);

            build_tray(&handle)?;

            if let Some(window) = handle.get_window("main") {
                let resize_handle = handle.clone();
                window.on_window_event(move |event| match event {
                    WindowEvent::Resized(_) => embed::apply_bounds(&resize_handle),
                    WindowEvent::Focused(focused) => {
                        // 窗口重新被激活，两件事都要做：
                        // 1. 通知前端把 DOM 焦点与光标补回具体元素 —— 只有前端知道用户
                        //    之前在编辑哪个输入框、光标停在哪；
                        // 2. Rust 侧把**系统级**键盘焦点交给该拿它的那个 webview（内嵌页面
                        //    显示着就归它，否则归主 webview）。不能只依赖前端那次 invoke
                        //    往返：系统把焦点还给谁与各 WebView2 各自要焦点都是异步的，
                        //    单发一次会被盖掉，必须事后校验、按几个时间点补刀（见 winfocus）。
                        //
                        // 注意这里**不判断**「内嵌页面是否显示着」：归属由 winfocus 按容器
                        // HWND 的真实显隐去问窗口本身，免得标志位和真实显隐不一致。
                        if *focused {
                            let _ = resize_handle.emit_to("main", FOCUSED_EVENT, ());
                            winfocus::claim_deferred(&resize_handle);
                        }
                    }
                    WindowEvent::CloseRequested { api, .. } => {
                        let close_action = resize_handle
                            .state::<StoreState>()
                            .settings()
                            .close_action;
                        if close_action != CLOSE_ACTION_EXIT {
                            // 兜底路径：Alt+F4 / 任务栏关闭时同样按「最小化到托盘」处理，
                            // 与顶栏关闭按钮保持完全一致。
                            api.prevent_close();
                            hide_to_tray(&resize_handle);
                        }
                    }
                    _ => {}
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load_store,
            save_store,
            set_close_action,
            app_info,
            check_port,
            start_service,
            stop_service,
            stop_all_services,
            running_services,
            open_terminal,
            open_embed,
            set_embed_insets,
            show_embed,
            hide_embeds,
            reload_embed,
            close_embed,
            show_tab_menu,
            window_minimize,
            window_toggle_maximize,
            window_is_maximized,
            window_close,
            focus_webview,
            open_external,
        ])
        .on_menu_event(|app, event| {
            appmenu::dispatch(app, event.id().as_ref());
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                let mgr = app.state::<ServiceManager>();
                service::stop_all(&mgr);
            }
        });
}
