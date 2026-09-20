//! 标签页右键的原生弹出菜单。
//!
//! 窗口本身的操作（设置 / 配置 / 查看启动日志 / 最小化 / 最大化 / 关闭）
//! 都在无边框窗口的自绘顶栏（`AppToolbar.vue`）里，Rust 侧只负责标签页
//! 的上下文菜单：`重载`、`重新启动`（仅本地启动的配置）、`打开终端`、
//! `在系统浏览器打开`。
//!
//! 菜单点击不在 Rust 侧实现业务，统一转成事件交给前端（前端才有标签页与配置的状态）。

use tauri::{
    menu::{ContextMenu, Menu, MenuItem, PredefinedMenuItem},
    AppHandle, Emitter, Manager,
};

/// 标签页右键菜单动作事件（Rust -> 主界面），payload 为 `{ tab, action }`
pub const EVT_TAB_MENU: &str = "menu://tab";

/// 菜单项 id 前缀（`tab:<action>:<tabId>`）
const TAB_PREFIX: &str = "tab:";

/// 处理菜单事件，把弹出菜单的点击翻译成前端动作。返回是否已消费。
pub fn dispatch(app: &AppHandle, id: &str) -> bool {
    let Some(rest) = id.strip_prefix(TAB_PREFIX) else {
        return false;
    };
    // tab:<action>:<tabId>，tabId 里允许再有冒号，所以只切第一个
    let Some((action, tab)) = rest.split_once(':') else {
        return false;
    };
    let _ = app.emit_to(
        "main",
        EVT_TAB_MENU,
        serde_json::json!({ "tab": tab, "action": action }),
    );
    true
}

fn tab_item(
    app: &AppHandle,
    action: &str,
    tab: &str,
    label: &str,
) -> Result<MenuItem<tauri::Wry>, String> {
    MenuItem::with_id(
        app,
        format!("{TAB_PREFIX}{action}:{tab}"),
        label,
        true,
        None::<&str>,
    )
    .map_err(|e| e.to_string())
}

/// 弹出标签页右键菜单。
///
/// 这里用**原生弹出菜单**而不是 HTML 浮层：标签栏下方是内嵌页面的 WebView2 子窗口
/// （独立的原生 HWND，永远盖在 HTML 之上），HTML 浮层伸出标签栏的部分会被它挡住。
///
/// 「打开终端」对任何标签页都可用：工作目录取该配置的工作目录，没有就开在启动器
/// 所在目录（非本地启动的配置同样如此）。
pub async fn popup_tab_menu(app: AppHandle, tab: String, can_restart: bool) -> Result<(), String> {
    let window = app.get_window("main").ok_or("主窗口不存在")?;
    let reload = tab_item(&app, "reload", &tab, "重载")?;
    let terminal = tab_item(&app, "terminal", &tab, "打开终端")?;
    let browser = tab_item(&app, "browser", &tab, "在系统浏览器打开")?;
    // 分隔线把「页面操作」（重载 / 重新启动）与「外部工具」（终端 / 浏览器）分开
    let separator = PredefinedMenuItem::separator(&app).map_err(|e| e.to_string())?;

    let menu = if can_restart {
        let restart = tab_item(&app, "restart", &tab, "重新启动")?;
        Menu::with_items(&app, &[&reload, &restart, &separator, &terminal, &browser])
    } else {
        Menu::with_items(&app, &[&reload, &separator, &terminal, &browser])
    }
    .map_err(|e| e.to_string())?;

    menu.popup(window).map_err(|e| e.to_string())
}
