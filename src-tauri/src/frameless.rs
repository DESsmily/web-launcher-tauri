//! 无边框窗口的 Windows 行为补丁。
//!
//! `decorations: false` 后窗口没有系统标题栏，但 Windows 处理最大化时
//! 默认把窗口铺满**整个监视器**（`WM_GETMINMAXINFO` 的默认 `ptMaxSize`），
//! 会盖住任务栏。这里用 `SetWindowSubclass` 拦截该消息：先走默认流程
//! （保留 tao 的最小 / 最大尺寸约束），再把最大化尺寸与位置改写为
//! **工作区**（work area，不含任务栏）。原生 maximize / unmaximize 的
//! 语义（还原边界、状态查询）都保持不变。

#[cfg(windows)]
pub fn install(app: &tauri::AppHandle) {
    use tauri::Manager;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{MINMAXINFO, WM_GETMINMAXINFO};

    const SUBCLASS_ID: usize = 0x57_4C_43_48; // "WLCH"

    unsafe extern "system" fn limit_maximize_to_work_area(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _uid: usize,
        _data: usize,
    ) -> LRESULT {
        if msg == WM_GETMINMAXINFO {
            // 先让默认处理跑完（尺寸约束、track size 都已就位），再覆盖最大化部分
            let result = DefSubclassProc(hwnd, msg, wparam, lparam);
            unsafe {
                let mmi = lparam as *mut MINMAXINFO;
                if !mmi.is_null() {
                    let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
                    if !monitor.is_null() {
                        let mut info: MONITORINFO = std::mem::zeroed();
                        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
                        if GetMonitorInfoW(monitor, &mut info) != 0 {
                            let work: RECT = info.rcWork;
                            let full: RECT = info.rcMonitor;
                            (*mmi).ptMaxSize = POINT {
                                x: work.right - work.left,
                                y: work.bottom - work.top,
                            };
                            // ptMaxPosition 相对监视器原点
                            (*mmi).ptMaxPosition = POINT {
                                x: work.left - full.left,
                                y: work.top - full.top,
                            };
                        }
                    }
                }
            }
            return result;
        }
        unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
    }

    let Some(window) = app.get_window("main") else {
        return;
    };
    let Ok(handle) = window.hwnd() else {
        return;
    };
    let hwnd = handle.0 as *mut core::ffi::c_void;

    unsafe {
        SetWindowSubclass(hwnd, Some(limit_maximize_to_work_area), SUBCLASS_ID, 0);
    }
}

#[cfg(not(windows))]
pub fn install(_app: &tauri::AppHandle) {}
