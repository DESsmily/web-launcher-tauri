use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{
    webview::NewWindowResponse, AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize,
    Position, Size, WebviewUrl,
};

/// 内嵌内容 webview 的 label 前缀：每个标签页对应一个独立的 WebView2 实例
pub const EMBED_PREFIX: &str = "embed-";

/// 页面标题变化事件（Rust -> 主界面），payload 为 `{ tab, title }`
pub const TITLE_EVENT: &str = "embed://title";

/// 内嵌页面实例被回收事件（Rust -> 主界面），payload 为 `{ tab }`。
///
/// 标签页本身还在（配置、日志、地址都不变），只是它背后的 webview 被销毁了，
/// 前端收到后要把该标签页标记成「未创建」，下次切回去时按需重建。
pub const EVICTED_EVENT: &str = "embed://evicted";

/// **同时存活的内嵌 webview 上限**，超过就按 LRU 销毁最久没用过的那个。
///
/// 为什么要设上限：每个标签页都是一个独立的 WebView2 控制器，各自带一个渲染进程
/// （实测一个普通开发页面的渲染进程在几十到几百 MB）。标签页只由用户手动关闭，
/// 于是「反复打开不同项目 / 长时间挂着」会让内嵌进程一路堆积 ——
/// 这正是「子进程把主程序一起拖崩」的来源。浏览器对后台标签页也是这么做的
/// （discard / 内存不足时回收），只是这里用一个固定的上限来兜。
///
/// 注意：代价是被回收的页面会丢失自身状态（表单、滚动位置、SPA 路由），
/// 切回去相当于重新打开一次。要调整资源占用与体验的平衡，改这一个常量即可。
const MAX_LIVE_EMBEDS: usize = 20;

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
    /// tab -> 最近一次使用序号（LRU 依据）。键集合同时充当「我认为还存活的页面」台账，
    /// 不依赖 `app.webviews()`：`Webview::close()` 是投递到事件循环异步执行的，
    /// 关完立刻回读运行时的表并不可靠。
    last_used: Mutex<HashMap<String, u64>>,
    clock: AtomicU64,
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

    /// 标记某标签页刚被使用过（打开 / 显示）
    fn touch(&self, tab: &str) {
        let tick = self.clock.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut map) = self.last_used.lock() {
            map.insert(tab.to_string(), tick);
        }
    }

    /// 页面实例已销毁（关闭标签页 / 被回收）时从台账里摘掉
    fn forget(&self, tab: &str) {
        if let Ok(mut map) = self.last_used.lock() {
            map.remove(tab);
        }
    }

    /// 为「即将新开 `keep_tab` 这个页面」算出需要回收哪些标签页（最久未用的排在前面）。
    ///
    /// 只算一次、返回离线名单，不在循环里重新回读运行时状态 —— 既避免异步 close
    /// 造成的死循环，也让整批回收是一次性的。
    fn eviction_plan(&self, keep_tab: &str) -> Vec<String> {
        let Ok(map) = self.last_used.lock() else {
            return Vec::new();
        };
        // keep_tab 不在台账里说明这是一次「新建」，总量会 +1
        let incoming = usize::from(!map.contains_key(keep_tab));
        let excess = (map.len() + incoming).saturating_sub(MAX_LIVE_EMBEDS);
        if excess == 0 {
            return Vec::new();
        }

        let mut candidates: Vec<(&String, u64)> = map
            .iter()
            .filter(|(tab, _)| tab.as_str() != keep_tab)
            .map(|(tab, tick)| (tab, *tick))
            .collect();
        candidates.sort_by_key(|(_, tick)| *tick);
        candidates
            .into_iter()
            .take(excess)
            .map(|(tab, _)| tab.clone())
            .collect()
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

/// 销毁最久未使用的内嵌页面，把「存活数量」压回上限以内。
///
/// 只能在**创建新页面之前**调用（`keep_tab` 是要新开的那个，不参与回收）。
/// 回收后发事件让前端把该标签页标记成「未创建」—— 标签页还在，切回去会重建。
fn evict_excess(app: &AppHandle, keep_tab: &str) {
    let Some(state) = app.try_state::<EmbedState>() else {
        return;
    };

    for tab in state.eviction_plan(keep_tab) {
        let label = label_for(&tab);
        if let Some(webview) = app.get_webview(&label) {
            // 先隐再关：正在显示 / 持有键盘焦点的子窗口直接销毁，焦点会悬在半空
            let _ = webview.hide();
            let _ = webview.close();
        }
        state.forget(&tab);
        let _ = app.emit_to("main", EVICTED_EVENT, serde_json::json!({ "tab": tab }));
    }
}

/// 主窗口当前是否真的可见。
///
/// 问 Win32 而不是 tao 的 `is_visible()`：后者读的是 tao 自己的内部标记，在「先
/// `ShowWindow` 再让 tao 追状态」的还原顺序下会短暂失真。`IsWindowVisible` 对顶层窗口
/// 返回的正是 `WS_VISIBLE` 的真实值。
#[cfg(windows)]
fn window_is_visible(app: &AppHandle) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::IsWindowVisible;

    let Some(window) = app.get_window("main") else {
        return false;
    };
    match window.hwnd() {
        Ok(handle) => unsafe { IsWindowVisible(handle.0 as *mut core::ffi::c_void) != 0 },
        // 拿不到句柄时按「可见」处理：宁可多做一次重排，也不要让内嵌页面永远停在旧尺寸
        Err(_) => true,
    }
}

#[cfg(not(windows))]
fn window_is_visible(_app: &AppHandle) -> bool {
    true
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

    // 主窗口不可见（隐藏到托盘，或正处在隐藏 / 还原的过渡里）时**不去动内嵌页面**。
    //
    // wry 的 `set_bounds` / `set_visible` 是同步 COM 调用：它们会 marshal 到 WebView2
    // 自己的 UI 线程，而那一边在窗口隐藏后并不保证能立刻响应。窗口隐藏时去重排内嵌
    // 页面本来就没有意义（看不见），却会把这些调用塞进主线程 —— 主线程同时也是
    // 托盘图标 / 原生菜单的消息泵所在，一旦被拖住，表现就是「点托盘没反应」。
    //
    // 因此这里按 Win32 的真实可见性提前返回；还原时 `embed::show()` 会再调一次
    // `apply_bounds`，那时窗口已经可见，尺寸照常算得出来。
    if !window_is_visible(app) {
        return;
    }

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

/// 注入内嵌页面的脚本：兜底两件事。
///
/// 1. 拦截 `window.open(...)` 并转成可识别的 `<a target="_blank">` 点击。
///    这是 Rust 侧 `on_new_window` 之外的兜底。页面里有些 `window.open` 调用因为
///    手势链被异步隔断，WebView2 不会触发 `NewWindowRequested`；把它转成一个
///    target="_blank" 的链接点击后，opener 插件自带的 JS 脚本会接管并打开系统浏览器。
///    `<a target="_blank">` 本身则由 opener 插件直接处理，这里不再重复监听，
///    避免双重打开或权限冲突。
/// 2. 记住用户最后聚焦的元素，并在窗口重新获得焦点时把它补回来。
///    Rust 侧只能把键盘焦点送到 webview 窗口这一层，页面里具体哪个元素持有
///    `activeElement` 它管不到（WebView2/页面自己可能在失焦后把它重置为 BODY 或
///    第一个元素）。这个兜底脚本与主界面的 `lib/focus.ts` 分工相同：只负责 embed
///    页面内的元素级焦点恢复。
const INIT_SCRIPT: &str = r#"
(function () {
  if (window.__WEBLAUNCH_JUMP_INSTALLED__) return;
  window.__WEBLAUNCH_JUMP_INSTALLED__ = true;

  function isExternalUrl(url) {
    if (typeof url !== 'string') return false;
    var trimmed = url.trim();
    if (!trimmed) return false;
    try {
      var u = new URL(trimmed, location.href);
      return u.protocol === 'http:' || u.protocol === 'https:' || u.protocol === 'file:';
    } catch (_) {
      return /^https?:\/\//i.test(trimmed) || /^file:/i.test(trimmed);
    }
  }

  function shouldInterceptOpen(target) {
    return target === '_blank' || target === undefined || target === null || target === '';
  }

  // 把外部新窗口请求转成 target="_blank" 的 a 标签点击，让 opener 插件的 JS 脚本接管
  function openInBrowser(url) {
    var a = document.createElement('a');
    a.href = url;
    a.target = '_blank';
    a.rel = 'noopener noreferrer';
    a.style.display = 'none';
    (document.body || document.documentElement).appendChild(a);
    a.click();
    setTimeout(function () { a.remove(); }, 0);
  }

  var originalOpen = window.open;
  window.open = function (url, target, features) {
    if (typeof url === 'string' && isExternalUrl(url) && shouldInterceptOpen(target)) {
      openInBrowser(url);
      return null;
    }
    return originalOpen.apply(this, arguments);
  };

  // --- 焦点恢复兜底 ---
  //
  // 只记「用户真的在编辑」的元素（input / textarea / select / contenteditable）。
  // **刻意不记 `<a>`**：浏览器/页面在「找回焦点但记不得先前元素」时会自己聚焦
  // 第一个可聚焦元素，而那常常就是页面左上角的 logo 链接；一旦把链接也记成目标，
  // 之后每次恢复都去聚焦它 —— 反馈放大，表现就是「焦点一直钉在左上角」。必须掐掉。
  var lastField = null;
  var restores = 0;
  var watchdog = 0;
  // 这个时刻之前都算「刚回到窗口」：被程序化抢走的焦点要立刻抢回来
  var watchdogUntil = 0;
  // 窗口失焦时置位（回来后、用户还没动作之前，任何「不是 lastField」的聚焦都当成被抢走）。
  // 用「失焦时置位」而不是「获得焦点时置位」：那个抢焦点的 focusin 可能比 window.focus 更早到。
  var pendingRestore = false;

  function isField(el) {
    if (!el || !(el instanceof HTMLElement)) return false;
    var tag = el.tagName;
    return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' ||
           el.isContentEditable === true;
  }

  function tryFocus(el) {
    if (!el || !el.isConnected || document.activeElement === el) return false;
    try {
      el.focus({ preventScroll: true });
    } catch (_) {
      try { el.focus(); } catch (_) {}
    }
    restores++;
    return true;
  }

  document.addEventListener('focusin', function (e) {
    // 「刚回到窗口」期间：任何不是 lastField 的聚焦都当成被**程序化**抢走
    // （WebView2 会把焦点丢给页面第一个可聚焦元素，常是左上角的 logo 链接），
    // 在**同一个事件循环里**立刻抢回来 —— 中间那一瞬间不会被画出来，所以看不到闪烁。
    if (pendingRestore || Date.now() < watchdogUntil) {
      if (lastField && e.target !== lastField && !lastField.contains(e.target)) {
        tryFocus(lastField);
        return;
      }
      if (!lastField && isField(e.target)) lastField = e.target;
      return;
    }
    if (isField(e.target)) lastField = e.target;
  }, true);

  // **刻意不主动清 `lastField`**（早先按「点到别处」清过）：页面 / WebView2 / 宿主的
  // 程序化焦点变化会把记忆误清，于是窗口回来时无目标可恢复、焦点又钉在第一个元素上。
  // 目标失效由 `isConnected` 挡掉就够了。

  function restoreFieldFocus() {
    return tryFocus(lastField);
  }

  function stopWatchdog() {
    watchdogUntil = 0;
    pendingRestore = false;
  }

  // 窗口重新获得焦点：页面/WebView2/宿主都可能把 activeElement 重置到 BODY 或第一个元素，
  // 所以在若干时间点各校验一次（幂等；拉到 1.5s 是为了覆盖「它们比我们晚」）。
  var RESTORE_DELAYS_MS = [0, 50, 150, 400, 800, 1500];

  // 窗口失焦：先把「待恢复」标记与守备窗口置上（`window.focus` 不一定先于那次
  // 「抢焦点」的 focusin 到达，靠它开窗口会漏）
  window.addEventListener('blur', function () {
    pendingRestore = true;
    watchdogUntil = Date.now() + 1500;
  });

  window.addEventListener('focus', function () {
    RESTORE_DELAYS_MS.forEach(function (ms) {
      window.setTimeout(restoreFieldFocus, ms);
    });
    // 再开一个短窗口的看门狗：高频校验兜底，配合上面的 focusin 处理消除闪烁
    watchdogUntil = Date.now() + 1500;
    if (!watchdog) {
      watchdog = window.setInterval(function () {
        if (Date.now() > watchdogUntil) {
          window.clearInterval(watchdog);
          watchdog = 0;
          return;
        }
        restoreFieldFocus();
      }, 40);
    }
  });

  // 用户一有动作就停手（此后焦点归他管，我们不抢）
  document.addEventListener('pointerdown', stopWatchdog, true);
  document.addEventListener('keydown', stopWatchdog, true);

  // 给宿主（Rust 的激活链）在最后再补一刀用：立即校验一次并回一段状态，便于诊断。
  window.__WEBLAUNCH_RESTORE_FOCUS__ = function () {
    restoreFieldFocus();
    var el = document.activeElement;
    return (document.hasFocus() ? 'focus=1' : 'focus=0') +
      ' active=' + (el ? el.tagName : 'null') +
      ' last=' + (lastField ? lastField.tagName : 'null') +
      ' restores=' + restores;
  };
})();
"#;

/// 页面请求「新窗口」时，判断是否该转交系统默认浏览器。
///
/// `window.open()` 不带参数、或页面自己往空文档里写内容的情况，WebView2 给的 Uri 是
/// `about:blank` —— 交出去只会弹出一个空白浏览器窗口，直接忽略。其余非浏览器 scheme
/// （`javascript:`、`data:`、`blob:`、`devtools:` 等）同样不往外抛。
fn open_in_browser(url: &str) {
    let trimmed = url.trim();
    let scheme = trimmed.split(':').next().unwrap_or("").to_ascii_lowercase();
    if !matches!(scheme.as_str(), "http" | "https" | "file" | "mailto") {
        return;
    }

    if let Err(error) = crate::open_in_system_browser(trimmed) {
        eprintln!("[embed] 系统浏览器打开失败：{error}（{trimmed}）");
    }
}

/// 在 LRU 台账里登记一次使用
fn touch(app: &AppHandle, tab: &str) {
    if let Some(state) = app.try_state::<EmbedState>() {
        state.touch(tab);
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
        touch(app, tab);
        apply_bounds(app);
        webview
            .navigate(parsed)
            .map_err(|e| format!("页面加载失败：{e}"))?;
        hide_others(app, tab);
        let _ = webview.show();
        // 同上：用 `SetFocus(容器)` 而不是 `Webview::set_focus()`（MoveFocus）。
        crate::winfocus::claim_visible(app);
        return Ok(());
    }

    // 新建之前先把存活数量压回上限：超出就销毁最久没用过的页面（标签页保留）。
    // 必须在 `add_child` 之前做，否则会先多出一个渲染进程再回收。
    evict_excess(app, tab);
    touch(app, tab);

    let window = app.get_window("main").ok_or("主窗口不存在")?;

    // 页面标题由 WebView2 的 DocumentTitleChanged 上报，无需注入脚本或 IPC
    let emitter = app.clone();
    let tab_id = tab.to_string();
    let builder = tauri::webview::WebviewBuilder::new(&label, WebviewUrl::External(parsed))
        // `focused: false` —— wry 的默认值是 `true`，那样**创建时**就会调一次
        // `controller.MoveFocus(PROGRAMMATIC)`（wry `webview2/mod.rs:546`），
        // 而 MoveFocus 会把页面 activeElement 定到「先前聚焦的元素，没有就第一个元素」，
        // 也就是「焦点跑到左上角」。键盘交给谁由 `winfocus::claim_visible` 用
        // `SetFocus(容器)` 决定，不需要 wry 代劳。
        .focused(false)
        // 注入脚本兜底：拦截 window.open / target="_blank" 点击，直接调 open_external
        .initialization_script(INIT_SCRIPT)
        .on_document_title_changed(move |_webview, title| {
            let _ = emitter.emit_to(
                "main",
                TITLE_EVENT,
                serde_json::json!({ "tab": tab_id.clone(), "title": title }),
            );
        })
        // 页面里「在新窗口打开」的跳转（`window.open` / `target="_blank"` /
        // 表单 `target`）一律转交**系统默认浏览器**，不再在内嵌环境里开第二个窗口。
        //
        // 不设这个 handler 时 wry 是**直接把请求吞掉**的：`new_window_req_handler`
        // 为 None 时只执行 `args.SetHandled(true)` 就返回（webview2/mod.rs），页面
        // 既不报错也拿不到任何反馈 —— 现象正是「点了跳转没反应」。所以这里必须显式
        // 接住：先交给系统浏览器，再返回 `Deny` 明确拒绝新建内嵌窗口。
        .on_new_window(|url, _features| {
            open_in_browser(url.as_str());
            NewWindowResponse::Deny
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
    // 不调 `webview.set_focus()`（= `MoveFocus(PROGRAMMATIC)`，会把页面 activeElement
    // 冲成第一个元素 → 焦点跳左上角）。改为走 winfocus 的 HWND 层：`SetFocus(容器)`。
    crate::winfocus::claim_visible(app);
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
    touch(app, tab);
    apply_bounds(app);
    hide_others(app, tab);
    let _ = webview.show();
    // 不调 `webview.set_focus()`（= `MoveFocus(PROGRAMMATIC)`，会把页面 activeElement
    // 冲成第一个元素 → 焦点跳左上角）。改为走 winfocus 的 HWND 层：`SetFocus(容器)`。
    crate::winfocus::claim_visible(app);
    Ok(())
}

/// 隐藏全部内嵌页面（进入配置 / 设置 / 编辑界面时）
///
/// 这里**不需要**额外通知谁「键盘该交还主界面了」：`hide()` 会把容器的 HWND 一并
/// `SW_HIDE`（wry 的 `set_visible`），而 `winfocus` 判断键盘归属时直接看容器的真实显隐。
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

/// 销毁某个标签页的内嵌页面并把它从 LRU 台账里摘掉。
///
/// 真正释放 WebView2 控制器（以及它那个渲染进程）发生在 wry 的
/// `InnerWebView::Drop`（webview2/mod.rs：`controller.Close()`）；tauri 的
/// `Webview::close()` 会把它从运行时窗口的 webview 列表里移除，引用计数归零后
/// Drop 才跑。所以这里必须先 `hide()` 再 `close()`，别让它带着键盘焦点被销毁。
pub fn close(app: &AppHandle, tab: &str) -> Result<(), String> {
    let label = label_for(tab);

    let result = match app.get_webview(&label) {
        Some(webview) => {
            let _ = webview.hide();
            webview.close().map_err(|e| e.to_string())
        }
        None => Ok(()),
    };

    if let Some(state) = app.try_state::<EmbedState>() {
        state.forget(tab);
    }
    result
}

/// LRU 回收的边界用例。`eviction_plan` 是纯计算（只读台账），所以能脱离窗口直接测。
///
/// 用例全部**按 `MAX_LIVE_EMBEDS` 推导**，不写死具体数字 —— 那个上限是给人调的旋钮
/// （改一个常量就能在「省资源」和「少丢页面状态」之间换挡），写死了测试就会在调参后
/// 莫名其妙地红掉。
#[cfg(test)]
mod tests {
    use super::*;

    /// 生成 `count` 个标签页 id：`t0` 最久没用过，`t{count-1}` 最近用过
    fn tab_ids(count: usize) -> Vec<String> {
        (0..count).map(|i| format!("t{i}")).collect()
    }

    /// 按顺序 touch 一串标签页，返回台账（前面的 = 更久没用过）
    fn state_of(tabs: &[String]) -> EmbedState {
        let state = EmbedState::default();
        for tab in tabs {
            state.touch(tab);
        }
        state
    }

    /// 台账里装满 `count` 个页面
    fn state_with(count: usize) -> EmbedState {
        state_of(&tab_ids(count))
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn 上限配置本身是合理的() {
        // 下面几个用例都假设「至少能留一个页面」；上限为 0 在应用里没有意义
        // （那就等于永远不保留内嵌页面），先把这个前提钉死。
        assert!(MAX_LIVE_EMBEDS > 0, "MAX_LIVE_EMBEDS 必须大于 0");
    }

    #[test]
    fn 没到上限不回收() {
        let state = state_with(MAX_LIVE_EMBEDS.saturating_sub(1));
        assert!(state.eviction_plan("new").is_empty());
    }

    #[test]
    fn 刚好满员时新建只回收最久未用的一个() {
        // 满员 + 1 个新页面 = 超 1 个 → 回收最久没用过的
        let state = state_with(MAX_LIVE_EMBEDS);
        assert_eq!(state.eviction_plan("new"), names(&["t0"]));
    }

    #[test]
    fn 重开已有标签页不触发回收() {
        // 已经在台账里 → 总量不变，不该回收任何东西
        let state = state_with(MAX_LIVE_EMBEDS);
        let existing = tab_ids(MAX_LIVE_EMBEDS)[MAX_LIVE_EMBEDS / 2].clone();
        assert!(state.eviction_plan(&existing).is_empty());
    }

    #[test]
    fn 被保留的标签页自己不会被回收() {
        // t0 最久没用过，但它就是本次要打开的那个 → 该回收的是紧随其后的两个
        let state = state_with(MAX_LIVE_EMBEDS + 2);
        assert_eq!(state.eviction_plan("t0"), names(&["t1", "t2"]));
    }

    #[test]
    fn 超员几个就回收几个() {
        // 满员 + 3 个新页面 → 一次回收最久没用过的三个
        let state = state_with(MAX_LIVE_EMBEDS + 2);
        assert_eq!(state.eviction_plan("new"), names(&["t0", "t1", "t2"]));
    }

    #[test]
    fn 关闭后从台账摘掉不再参与回收() {
        let state = state_with(MAX_LIVE_EMBEDS + 1);
        state.forget("t0");
        // t0 已经被销毁（关闭标签页），回收名单里不该再出现它
        assert_eq!(state.eviction_plan("new"), names(&["t1"]));
    }

    #[test]
    fn 关闭后腾出的名额让新建不再回收() {
        let state = state_with(MAX_LIVE_EMBEDS);
        state.forget("t0");
        assert!(state.eviction_plan("new").is_empty());
    }
}
