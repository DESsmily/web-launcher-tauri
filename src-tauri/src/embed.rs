use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, Position, Size, WebviewUrl,
};

/// 内嵌内容 webview 的 label 前缀：每个标签页对应一个独立的 WebView2 实例
pub const EMBED_PREFIX: &str = "embed-";

/// 页面标题变化事件（Rust -> 主界面），payload 为 `{ tab, title }`
pub const TITLE_EVENT: &str = "embed://title";

/// 标签页 id -> 内嵌 webview label
pub fn label_for(tab: &str) -> String {
    format!("{EMBED_PREFIX}{tab}")
}

/// 内容区域相对主窗口的留白（逻辑像素），用于给顶部工具栏占位
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Insets {
    pub x: f64,
    pub y: f64,
    pub right: f64,
    pub bottom: f64,
}

#[derive(Default)]
pub struct EmbedState {
    insets: Mutex<Insets>,
}

impl EmbedState {
    fn set(&self, insets: Insets) {
        if let Ok(mut guard) = self.insets.lock() {
            *guard = insets;
        }
    }

    fn get(&self) -> Option<Insets> {
        self.insets.lock().ok().map(|guard| *guard)
    }
}

/// 当前存在的全部内嵌内容 webview（`webviews()` 返回的是 `(label, webview)` 元组）
fn embeds(app: &AppHandle) -> Vec<tauri::Webview> {
    app.webviews()
        .into_iter()
        .filter(|(label, _)| label.starts_with(EMBED_PREFIX))
        .map(|(_, webview)| webview)
        .collect()
}

/// 隐藏除指定标签页以外的全部内嵌页面
fn hide_others(app: &AppHandle, keep_tab: &str) {
    let keep = label_for(keep_tab);
    for webview in embeds(app) {
        if webview.label() != keep {
            let _ = webview.hide();
        }
    }
}

/// 主窗口客户区尺寸（物理像素）。
///
/// 优先问 Win32 要 `GetClientRect`：原生菜单栏是**非客户区**，它会占掉窗口顶部一条，
/// 客户区（也就是子 webview 的坐标系原点所在的区域）随之下移。直接取真实矩形最稳。
fn client_size(app: &AppHandle) -> Option<(u32, u32)> {
    let window = app.get_window("main")?;

    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::RECT;
        use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;

        if let Ok(hwnd) = window.hwnd() {
            let hwnd = hwnd.0 as *mut core::ffi::c_void;
            let mut rect: RECT = unsafe { std::mem::zeroed() };
            if unsafe { GetClientRect(hwnd, &mut rect) } != 0 {
                let width = (rect.right - rect.left).max(1) as u32;
                let height = (rect.bottom - rect.top).max(1) as u32;
                return Some((width, height));
            }
        }
    }

    window.inner_size().ok().map(|size| (size.width, size.height))
}

/// 让内容区铺满工具栏以下的空间。
///
/// 这里按**物理像素**换算而不是逻辑像素：逻辑像素要经历一次
/// `round(logical * scale)` 的反向取整，在高 DPI（125% / 150%）下会留下 1px 缝隙。
pub fn apply_bounds(app: &AppHandle) {
    let Some(state) = app.try_state::<EmbedState>() else {
        return;
    };
    let Some(insets) = state.get() else {
        return;
    };
    let Some(window) = app.get_window("main") else {
        return;
    };
    let Some((inner_width, inner_height)) = client_size(app) else {
        return;
    };

    let scale = window.scale_factor().unwrap_or(1.0);
    let left = (insets.x * scale).round() as i32;
    let top = (insets.y * scale).round() as i32;
    let right = (insets.right * scale).round() as i32;
    let bottom = (insets.bottom * scale).round() as i32;

    let width = (inner_width as i32 - left - right).max(1) as u32;
    let height = (inner_height as i32 - top - bottom).max(1) as u32;

    for webview in embeds(app) {
        let _ = webview.set_position(Position::Physical(PhysicalPosition::new(left, top)));
        let _ = webview.set_size(Size::Physical(PhysicalSize::new(width, height)));
    }
}

/// 创建（或复用）某个标签页的内嵌 webview 并加载地址
pub fn open(app: &AppHandle, tab: &str, url: &str, insets: Insets) -> Result<(), String> {
    let parsed = tauri::Url::parse(url).map_err(|e| format!("访问地址无效：{e}"))?;

    if let Some(state) = app.try_state::<EmbedState>() {
        state.set(insets);
    }

    let label = label_for(tab);

    // 复用已有实例：直接导航，保留该标签页的回退栈
    if let Some(webview) = app.get_webview(&label) {
        apply_bounds(app);
        webview
            .navigate(parsed)
            .map_err(|e| format!("页面加载失败：{e}"))?;
        hide_others(app, tab);
        let _ = webview.show();
        let _ = webview.set_focus();
        return Ok(());
    }

    let window = app.get_window("main").ok_or("主窗口不存在")?;

    // 页面标题由 WebView2 的 DocumentTitleChanged 上报，无需注入脚本或 IPC
    let emitter = app.clone();
    let tab_id = tab.to_string();
    let builder = tauri::webview::WebviewBuilder::new(&label, WebviewUrl::External(parsed))
        .on_document_title_changed(move |_webview, title| {
            let _ = emitter.emit_to(
                "main",
                TITLE_EVENT,
                serde_json::json!({ "tab": tab_id.clone(), "title": title }),
            );
        });

    let webview = window
        .add_child(
            builder,
            Position::Physical(PhysicalPosition::new(0, 0)),
            Size::Physical(PhysicalSize::new(1, 1)),
        )
        .map_err(|e| format!("创建内嵌页面失败：{e}"))?;

    let _ = webview.set_auto_resize(false);
    apply_bounds(app);
    hide_others(app, tab);
    let _ = webview.show();
    let _ = webview.set_focus();
    Ok(())
}

/// 更新留白并重新排布全部内嵌页面
pub fn set_insets(app: &AppHandle, insets: Insets) {
    if let Some(state) = app.try_state::<EmbedState>() {
        state.set(insets);
    }
    apply_bounds(app);
}

/// 显示指定标签页的页面，其余隐藏
pub fn show(app: &AppHandle, tab: &str) -> Result<(), String> {
    let label = label_for(tab);
    let Some(webview) = app.get_webview(&label) else {
        return Err("该标签页的页面尚未创建".to_string());
    };
    apply_bounds(app);
    hide_others(app, tab);
    let _ = webview.show();
    let _ = webview.set_focus();
    Ok(())
}

/// 隐藏全部内嵌页面（进入配置 / 设置 / 编辑界面时）
pub fn hide_all(app: &AppHandle) -> Result<(), String> {
    for webview in embeds(app) {
        let _ = webview.hide();
    }
    Ok(())
}

pub fn reload(app: &AppHandle, tab: &str) -> Result<(), String> {
    let label = label_for(tab);
    if let Some(webview) = app.get_webview(&label) {
        webview.reload().map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn close(app: &AppHandle, tab: &str) -> Result<(), String> {
    let label = label_for(tab);
    if let Some(webview) = app.get_webview(&label) {
        webview.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}
