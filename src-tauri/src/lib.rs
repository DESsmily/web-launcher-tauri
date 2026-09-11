mod appmenu;
mod embed;
mod frameless;
mod service;
mod store;

use std::process::Command;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, RunEvent, State, WindowEvent,
};

use embed::Insets;
use service::ServiceManager;
use store::{Store, StoreState, CLOSE_ACTION_EXIT};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

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

/// 窗口控制按钮（顶栏右侧的最小化 / 最大化 / 关闭）走这些命令。
/// 关闭必须走 `Window::close()` 以触发 `CloseRequested`，
/// 让「最小化托盘 / 关闭应用程序」的关闭策略在窗口事件里统一生效。
fn main_window(app: &AppHandle) -> Result<tauri::Window, String> {
    app.get_window("main").ok_or_else(|| "主窗口不存在".to_string())
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

#[tauri::command]
fn window_close(app: AppHandle) -> Result<(), String> {
    main_window(&app)?.close().map_err(|e| e.to_string())
}

#[tauri::command]
async fn show_tab_menu(app: AppHandle, tab: String, can_restart: bool) -> Result<(), String> {
    appmenu::popup_tab_menu(app, tab, can_restart).await
}

/* ---------------------------------- 系统能力 ---------------------------------- */

#[tauri::command]
fn open_external(url: String) -> Result<(), String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("地址为空".to_string());
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = Command::new("cmd.exe");
        cmd.args(["/D", "/C", "start", "", url])
            .creation_flags(CREATE_NO_WINDOW);
        cmd.spawn()
            .map(|_| ())
            .map_err(|e| format!("调用系统浏览器失败：{e}"))
    }
    #[cfg(not(windows))]
    {
        let mut cmd = Command::new("xdg-open");
        cmd.arg(url);
        cmd.spawn()
            .map(|_| ())
            .map_err(|e| format!("调用系统浏览器失败：{e}"))
    }
}

/* ------------------------------------ 托盘 ----------------------------------- */

fn focus_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let show_item = MenuItem::with_id(app, "show", "显示主界面", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, "quit", "退出程序", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_item, &separator, &quit_item])?;

    let mut builder = TrayIconBuilder::with_id("web-launch-tray")
        .menu(&menu)
        .tooltip("Web 启动器");

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    builder
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => focus_main(app),
            "quit" => {
                let mgr = app.state::<ServiceManager>();
                service::stop_all(&mgr);
                app.exit(0);
            }
            _ => {}
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
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(ServiceManager::default())
        .manage(embed::EmbedState::default())
        .setup(|app| {
            let handle = app.handle().clone();
            app.manage(StoreState::new(&handle));

            // 无边框窗口补丁：最大化限制到工作区，避免盖住任务栏
            frameless::install(&handle);

            build_tray(&handle)?;

            if let Some(window) = handle.get_window("main") {
                let resize_handle = handle.clone();
                window.on_window_event(move |event| match event {
                    WindowEvent::Resized(_) => embed::apply_bounds(&resize_handle),
                    WindowEvent::CloseRequested { api, .. } => {
                        let close_action = resize_handle
                            .state::<StoreState>()
                            .settings()
                            .close_action;
                        if close_action != CLOSE_ACTION_EXIT {
                            api.prevent_close();
                            if let Some(window) = resize_handle.get_webview_window("main") {
                                let _ = window.hide();
                            }
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
