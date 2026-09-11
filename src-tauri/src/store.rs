use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

/// 关闭窗口时的行为
pub const CLOSE_ACTION_TRAY: &str = "tray";
pub const CLOSE_ACTION_EXIT: &str = "exit";

fn default_close_action() -> String {
    CLOSE_ACTION_TRAY.to_string()
}

fn default_timeout() -> u64 {
    120
}

fn default_probe_interval() -> u64 {
    500
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    /// tray = 最小化到托盘，exit = 直接退出应用
    #[serde(default = "default_close_action")]
    pub close_action: String,
    /// 等待本地服务启动的最长秒数
    #[serde(default = "default_timeout")]
    pub launch_timeout_secs: u64,
    /// 端口探测间隔（毫秒）
    #[serde(default = "default_probe_interval")]
    pub probe_interval_ms: u64,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            close_action: default_close_action(),
            launch_timeout_secs: default_timeout(),
            probe_interval_ms: default_probe_interval(),
        }
    }
}

impl AppSettings {
    pub fn normalized(mut self) -> Self {
        if self.close_action != CLOSE_ACTION_EXIT {
            self.close_action = CLOSE_ACTION_TRAY.to_string();
        }
        if self.launch_timeout_secs == 0 || self.launch_timeout_secs > 3600 {
            self.launch_timeout_secs = default_timeout();
        }
        if self.probe_interval_ms < 100 || self.probe_interval_ms > 10_000 {
            self.probe_interval_ms = default_probe_interval();
        }
        self
    }
}

fn default_local_start() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchConfig {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// 访问地址，例如 http://127.0.0.1:3000
    #[serde(default)]
    pub url: String,
    /// 是否需要由本应用通过 cmd 拉起本地服务
    #[serde(default = "default_local_start")]
    pub local_start: bool,
    /// 启动命令
    #[serde(default)]
    pub command: String,
    /// 命令工作目录（可选）
    #[serde(default)]
    pub cwd: String,
    /// 是否默认启动（全局互斥）
    #[serde(default)]
    pub is_default: bool,
    /// 备注
    #[serde(default)]
    pub remark: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Store {
    #[serde(default)]
    pub settings: AppSettings,
    #[serde(default)]
    pub configs: Vec<LaunchConfig>,
}

pub struct StoreState {
    pub path: PathBuf,
    pub current: Mutex<Store>,
}

impl StoreState {
    pub fn new(app: &AppHandle) -> Self {
        let dir = app
            .path()
            .app_config_dir()
            .unwrap_or_else(|_| PathBuf::from("."));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("store.json");

        let current = match std::fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str::<Store>(&raw)
                .map(|s| Store {
                    settings: s.settings.normalized(),
                    configs: s.configs,
                })
                .unwrap_or_default(),
            Err(_) => Store::default(),
        };

        Self {
            path,
            current: Mutex::new(current),
        }
    }

    pub fn settings(&self) -> AppSettings {
        self.current
            .lock()
            .map(|s| s.settings.clone())
            .unwrap_or_default()
            .normalized()
    }

    pub fn read(&self) -> Store {
        let mut store = self
            .current
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default();
        store.settings = store.settings.normalized();
        store
    }

    pub fn write(&self, store: Store) -> Result<(), String> {
        let mut store = store;
        store.settings = store.settings.normalized();
        // 默认启动配置保证唯一
        let mut default_seen = false;
        for cfg in store.configs.iter_mut() {
            if cfg.is_default {
                if default_seen {
                    cfg.is_default = false;
                } else {
                    default_seen = true;
                }
            }
        }

        let json = serde_json::to_string_pretty(&store).map_err(|e| e.to_string())?;
        std::fs::write(&self.path, json).map_err(|e| e.to_string())?;

        if let Ok(mut guard) = self.current.lock() {
            *guard = store;
        }
        Ok(())
    }
}
