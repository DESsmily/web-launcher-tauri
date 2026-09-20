use std::collections::HashMap;
use std::io::Read;
use std::net::{Ipv6Addr, SocketAddr, TcpStream};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 端口探测超时
const PROBE_TIMEOUT: Duration = Duration::from_millis(400);

pub const EVT_LOG: &str = "service://log";
pub const EVT_EXIT: &str = "service://exit";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogPayload {
    pub id: String,
    pub line: String,
    pub stream: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExitPayload {
    pub id: String,
    pub code: Option<i32>,
}

/// 由本应用通过 cmd 拉起的服务进程表：config id -> pid
#[derive(Default)]
pub struct ServiceManager {
    running: Arc<Mutex<HashMap<String, u32>>>,
}

impl ServiceManager {
    pub fn list(&self) -> Vec<String> {
        self.running
            .lock()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    }
}

/// 判断本地端口是否已经被监听（同时探测 IPv4 / IPv6）
pub fn check_port(port: u16) -> bool {
    if port == 0 {
        return false;
    }
    let v4 = SocketAddr::from(([127, 0, 0, 1], port));
    if TcpStream::connect_timeout(&v4, PROBE_TIMEOUT).is_ok() {
        return true;
    }
    let v6 = SocketAddr::from((Ipv6Addr::LOCALHOST, port));
    TcpStream::connect_timeout(&v6, PROBE_TIMEOUT).is_ok()
}

#[cfg(windows)]
fn shell(full_command: &str) -> Command {
    use std::os::windows::process::CommandExt;
    let mut cmd = Command::new("cmd.exe");
    // /D 跳过 AutoRun，/S 保留内层引号，/C 执行完退出
    cmd.raw_arg(format!("/D /S /C \"{}\"", full_command));
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

#[cfg(not(windows))]
fn shell(full_command: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(full_command);
    cmd
}

fn emit_line(app: &AppHandle, id: &str, stream: &str, bytes: &[u8]) {
    let raw = String::from_utf8_lossy(bytes);
    let text = raw.trim_end_matches(&['\r', '\n'][..]).to_string();
    if text.is_empty() {
        return;
    }
    let _ = app.emit(
        EVT_LOG,
        LogPayload {
            id: id.to_string(),
            line: text,
            stream: stream.to_string(),
        },
    );
}

fn pump<R: Read + Send + 'static>(app: AppHandle, id: String, mut reader: R, stream: &'static str) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        let mut pending: Vec<u8> = Vec::new();
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    pending.extend_from_slice(&buf[..n]);
                    while let Some(pos) = pending.iter().position(|b| *b == b'\n') {
                        let line: Vec<u8> = pending.drain(..=pos).collect();
                        emit_line(&app, &id, stream, &line);
                    }
                }
                Err(_) => break,
            }
        }
        if !pending.is_empty() {
            emit_line(&app, &id, stream, &pending);
        }
    });
}

fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill")
            .raw_arg(format!("/PID {} /T /F", pid))
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill")
            .args(["-9", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

pub fn start(
    app: &AppHandle,
    mgr: &ServiceManager,
    id: &str,
    command: &str,
    cwd: &str,
) -> Result<u32, String> {
    let command = command.trim();
    if command.is_empty() {
        return Err("启动命令为空".to_string());
    }

    // 同一个配置重复启动时，先回收旧进程
    stop(mgr, id);

    #[cfg(windows)]
    let full = format!("chcp 65001>nul & {}", command);
    #[cfg(not(windows))]
    let full = command.to_string();

    let mut cmd = shell(&full);
    let cwd = cwd.trim();
    if !cwd.is_empty() {
        cmd.current_dir(cwd);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("启动命令执行失败：{e}"))?;
    let pid = child.id();

    if let Some(out) = child.stdout.take() {
        pump(app.clone(), id.to_string(), out, "stdout");
    }
    if let Some(err) = child.stderr.take() {
        pump(app.clone(), id.to_string(), err, "stderr");
    }

    if let Ok(mut map) = mgr.running.lock() {
        map.insert(id.to_string(), pid);
    }

    let waiter_app = app.clone();
    let waiter_id = id.to_string();
    let running = mgr.running.clone();
    std::thread::spawn(move || {
        let code = child.wait().ok().and_then(|status| status.code());
        if let Ok(mut map) = running.lock() {
            if map.get(&waiter_id).copied() == Some(pid) {
                map.remove(&waiter_id);
            }
        }
        let _ = waiter_app.emit(
            EVT_EXIT,
            ExitPayload {
                id: waiter_id,
                code,
            },
        );
    });

    Ok(pid)
}

pub fn stop(mgr: &ServiceManager, id: &str) {
    let pid = mgr
        .running
        .lock()
        .ok()
        .and_then(|mut map| map.remove(id));
    if let Some(pid) = pid {
        kill_tree(pid);
    }
}

pub fn stop_all(mgr: &ServiceManager) {
    let pids: Vec<u32> = mgr
        .running
        .lock()
        .map(|mut map| {
            let v: Vec<u32> = map.values().copied().collect();
            map.clear();
            v
        })
        .unwrap_or_default();
    for pid in pids {
        kill_tree(pid);
    }
}

/* --------------------------------- 打开终端 --------------------------------- */

/// 校验工作目录：留空表示用默认目录，填了就必须真实存在。
///
/// 目录不存在时宁可报错也不要静默退回默认目录 —— 用户点「打开终端」就是冲着
/// 项目目录去的，悄悄开在别处反而更难发现。
fn resolve_cwd(cwd: Option<&str>) -> Result<Option<&str>, String> {
    let Some(dir) = cwd.map(str::trim).filter(|dir| !dir.is_empty()) else {
        return Ok(None);
    };
    if !std::path::Path::new(dir).is_dir() {
        return Err(format!("工作目录不存在：{dir}"));
    }
    Ok(Some(dir))
}

/// 打开一个 cmd 终端窗口，工作目录为 `cwd`（为空则用启动器自身的工作目录）。
///
/// 几个要点：
/// - 目录走 `Command::current_dir` 而**不是**拼进命令行，路径里的空格 / 中文 / `&`
///   都不必再操心转义（对照 `shell()` 那条走 cmd 解析的老路，那边必须小心引号）。
/// - `CREATE_NEW_CONSOLE`：启动器是 GUI 程序，本身没有控制台，新开的 cmd 必须自己
///   带一个窗口，否则进程起来了却看不见。
/// - `/D` 跳过 AutoRun 注册表项，`/S` 让 cmd 按规则剥掉外层引号，`/K` 执行后保留窗口。
///   带上 `chcp 65001` 与启动流程保持一致：node / npm / vite 这类工具输出的是 UTF-8，
///   控制台停在默认的 936 时中文会乱码。
#[cfg(windows)]
pub fn open_terminal(cwd: Option<&str>) -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

    let dir = resolve_cwd(cwd)?;

    let mut cmd = Command::new("cmd.exe");
    cmd.arg("/D").arg("/S").arg("/K").arg("chcp 65001>nul");
    if let Some(dir) = dir {
        cmd.current_dir(dir);
    }
    cmd.creation_flags(CREATE_NEW_CONSOLE);

    cmd.spawn().map_err(|e| format!("打开终端失败：{e}"))?;
    Ok(())
}

#[cfg(not(windows))]
pub fn open_terminal(cwd: Option<&str>) -> Result<(), String> {
    resolve_cwd(cwd)?;
    Err("打开终端目前仅支持 Windows".to_string())
}
