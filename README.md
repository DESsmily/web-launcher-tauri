# Web 启动器（web-launcher）

基于 **Tauri 2 + Vue 3 + TypeScript** 的桌面启动器：把需要本地服务支撑的 Web 页面集中管理，
一键检测端口 / 执行 cmd 启动命令，并用 **WebView2** 内嵌打开页面。

## 图片参考

![Web Launcher](/outputs/example1.png)
![Web Launcher](/outputs/example2.png)
![Web Launcher](/outputs/example3.png)

## 功能

### 启动配置列表（主界面）
- 新增 / 编辑 / 删除配置
- 一键「启动」、「设为默认启动」、查看运行状态
- 已启动的服务在卡片上标记为 **运行中**

### 配置项

| 配置 | 说明 |
| --- | --- |
| 配置名称 | 列表展示名 |
| 访问地址 | 如 `http://127.0.0.1:3000`，必须带 `http://` / `https://` |
| 是否本地启动 | **默认开启**。开启后显示「cmd 启动命令」输入框 |
| cmd 启动命令 | 如 `npm run dev`，在 Windows cmd 中执行并实时捕获输出 |
| 工作目录 | 可选，留空则在启动器所在目录执行 |
| 是否默认启动 | 开启后应用启动时自动打开；**全局唯一**，设置某一个后其他配置自动关闭 |
| 备注 | 可选 |

### 启动流程

1. **未开启本地启动** → 直接打开访问地址。
2. **开启本地启动** → 先解析端口（`:3000` → 3000，无端口时 http=80 / https=443）：
   - 端口已在监听 → 直接打开页面，不重复执行命令；
   - 端口未监听 → 执行 cmd 启动命令，实时显示输出，等待服务就绪后再打开页面。
3. 命令输出中若出现本机访问地址（如 `http://127.0.0.1:3080?token=xxxx`），**优先使用该地址**打开。

### 页面渲染

目标页面通过 **Microsoft Edge WebView2** 以子 WebView 的形式内嵌在主窗口内容区，
顶部保留启动器工具栏：返回、刷新、查看启动日志、用系统浏览器打开、设置。

### 设置

- **关闭操作**：`最小化到托盘`（默认）/ `关闭应用程序` 二选一
- 启动等待超时、端口探测间隔
- 版本信息（应用版本 / Tauri 版本 / 渲染内核）

### 进程生命周期

- 所有由启动器通过 cmd 拉起的进程都会被记录；
- 退出应用（或托盘菜单「退出程序」）时，自动结束全部由启动器拉起的进程树。

## 目录结构

```
src/                          前端（Vue 3 + TS）
  components/AppIcon.vue        内置图标
  components/ConfirmDialog.vue  确认弹窗
  lib/store.ts                  配置与设置的持久化
  lib/launch.ts                 启动流程 / 端口解析 / 输出地址提取
  lib/session.ts                视图路由与启动会话状态
  views/HomeView.vue            配置列表
  views/ConfigEditor.vue        新增 / 编辑配置
  views/SettingsView.vue        设置
  views/LaunchView.vue          启动视图（内嵌 WebView2）
src-tauri/                    后端（Rust）
  src/store.rs                  配置读写（store.json）
  src/service.rs                cmd 进程管理、端口探测、输出捕获
  src/embed.rs                  子 WebView2 的创建 / 定位 / 显隐
  src/lib.rs                    命令注册、托盘、关闭策略
```

配置文件位置：`%APPDATA%\com.weblaunch.app\store.json`

## 开发与构建

```bash
pnpm install
pnpm tauri dev      # 开发模式
pnpm tauri build    # 打包发布
```

要求：Node 18+、Rust 1.77+、Windows 上需安装 WebView2 Runtime（Win10 1803+ 通常自带）。

## 说明

- 内嵌 WebView 依赖 Tauri 的 `unstable` 特性（`WebviewBuilder` / `Window::add_child`），
  已在 `src-tauri/Cargo.toml` 中开启。
- 内嵌窗口是独立的 WebView2 实例而非 iframe，站点设置 `X-Frame-Options` 不会阻止它被嵌入；
  若站点自身有额外拦截，可用工具栏的「用系统浏览器打开」。


