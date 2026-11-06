use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use windows::core::{w, BOOL, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, GetCapture, VK_LBUTTON};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowExW, FindWindowW, GetClassNameW, GetWindowLongPtrW, IsIconic, IsWindow,
    IsWindowVisible, SendMessageW, SetParent, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    GWL_EXSTYLE, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_RESTORE,
    SW_SHOWNA, WS_EX_TOOLWINDOW,
};

// 壁纸层 WorkerW 的句柄（默认嵌入宿主），枚举回调里写，枚举结束后读
static WORKERW: Mutex<Option<isize>> = Mutex::new(None);
static DEFVIEW_FOUND: AtomicBool = AtomicBool::new(false);
// 桌面图标层 SHELLDLL_DefView 的直接宿主（Progman 或挂 DefView 的 WorkerW），仅实验用。
// Shell 的 Win+D 会跳过它，但桌面窗口过程会吞掉子窗口的鼠标消息，窗口可见却无法交互。
static DESKTOP_HOST: Mutex<Option<isize>> = Mutex::new(None);
// 本次实际 SetParent 的父句柄，守护协程据此判断父链是否被 Explorer 重建
static EMBED_PARENT: Mutex<Option<isize>> = Mutex::new(None);
// 当前是否处于嵌入模式；Win+D 守护协程据此决定要不要唤回
static EMBEDDED: AtomicBool = AtomicBool::new(false);
// 用户主动隐藏（托盘开关/点关闭/点最小化）时置位，守护协程见到就放手，否则会和主人抢窗口
pub static USER_HIDDEN: AtomicBool = AtomicBool::new(false);
// 异常状态只打一次日志，避免 200ms 体检把日志刷爆
static GUARD_LOGGED: AtomicBool = AtomicBool::new(false);

// 标记与最小化必须在同一次调用里完成，否则守护可能在两者之间抢先复位标记
#[tauri::command]
pub fn user_minimize(window: tauri::Window) {
    USER_HIDDEN.store(true, Ordering::Relaxed);
    let _ = window.minimize();
}

fn get_hwnd(window: &tauri::Window) -> Result<HWND, String> {
    window.hwnd().map_err(|e| format!("获取 HWND 失败: {e}"))
}

fn class_name_of(hwnd: HWND) -> String {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n.min(64)])
}

extern "system" fn enum_proc(hwnd: HWND, _lparam: LPARAM) -> BOOL {
    let class = class_name_of(hwnd);
    if class == "WorkerW" {
        // 桌面图标层（SHELLDLL_DefView）所在窗口之后出现的第一个 WorkerW 就是壁纸层
        if DEFVIEW_FOUND.load(Ordering::Relaxed) {
            let mut t = WORKERW.lock().unwrap();
            if t.is_none() {
                *t = Some(hwnd.0 as isize);
                return BOOL(0); // 找到了，停止枚举
            }
        }
        return BOOL(1);
    }
    // 检查该窗口是否挂着桌面图标层
    if let Ok(child) = unsafe { FindWindowExW(Some(hwnd), None, w!("SHELLDLL_DefView"), None) }
        && !child.0.is_null()
    {
        DEFVIEW_FOUND.store(true, Ordering::Relaxed);
    }
    BOOL(1)
}

fn find_workerw() -> Option<HWND> {
    let progman = unsafe { FindWindowW(w!("Progman"), PCWSTR::null()) }.ok()?;
    // 0x052C：让 Explorer 分裂出图标层背后的 WorkerW
    unsafe { SendMessageW(progman, 0x052C, Some(WPARAM(0)), Some(LPARAM(0))) };
    DEFVIEW_FOUND.store(false, Ordering::Relaxed);
    *WORKERW.lock().unwrap() = None;
    unsafe { let _ = EnumWindows(Some(enum_proc), LPARAM(0)); };
    let v = *WORKERW.lock().unwrap();
    v.map(|p| HWND(p as *mut _))
}

// 枚举顶层窗口，找「直接挂着 SHELLDLL_DefView」的那一个（Progman 或 WorkerW）。
// 这层只在 GHR_DESKTOP_HOST 实验开关下使用；默认壁纸层由守护协程在 Win+D 后唤回。
extern "system" fn enum_desktop_host_proc(hwnd: HWND, _lparam: LPARAM) -> BOOL {
    if let Ok(defview) = unsafe { FindWindowExW(Some(hwnd), None, w!("SHELLDLL_DefView"), None) }
        && !defview.0.is_null()
    {
        *DESKTOP_HOST.lock().unwrap() = Some(hwnd.0 as isize);
        return BOOL(0); // 找到了，停止枚举
    }
    BOOL(1)
}

fn find_desktop_host() -> Option<HWND> {
    *DESKTOP_HOST.lock().unwrap() = None;
    unsafe { let _ = EnumWindows(Some(enum_desktop_host_proc), LPARAM(0)); };
    let v = *DESKTOP_HOST.lock().unwrap();
    v.map(|p| HWND(p as *mut _))
}

fn embed_parent() -> Option<HWND> {
    let v = *EMBED_PARENT.lock().unwrap();
    v.map(|p| HWND(p as *mut _))
}

// 默认挂壁纸层 WorkerW：桌面图标层的宿主（Progman）是 Shell 的桌面窗口，
// 它的窗口过程会吞掉子窗口的鼠标激活消息，窗口看得见却完全无法交互。
// GHR_DESKTOP_HOST 才启用图标层宿主做严格 Win+D 免疫实验；GHR_WALLPAPER_LAYER 可强制回退。
fn prefer_desktop_host() -> bool {
    std::env::var("GHR_DESKTOP_HOST").is_ok() && std::env::var("GHR_WALLPAPER_LAYER").is_err()
}

fn choose_host(
    desktop: Option<HWND>,
    workerw: Option<HWND>,
    prefer_desktop: bool,
) -> Option<HWND> {
    if prefer_desktop {
        desktop.or(workerw)
    } else {
        workerw.or(desktop)
    }
}

fn find_host() -> Option<HWND> {
    let prefer = prefer_desktop_host();
    let desktop = if prefer { find_desktop_host() } else { None };
    // 首选已命中就别再发 0x052C 动 Explorer
    let workerw = if desktop.is_some() {
        None
    } else {
        find_workerw()
    };
    choose_host(desktop, workerw, prefer)
}

// 鼠标按下期间绝不唤回：SetWindowPos 会打断按下/抬起配对，让点击和拖拽全部失效
fn mouse_busy() -> bool {
    unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0 || !GetCapture().0.is_null() }
}

// 复用的展示动作：子窗口没有 minimized/maximized 概念，SW_RESTORE 对它无效，
// 要用 SW_SHOWNA + SWP_SHOWWINDOW；顶层窗口被最小化则由调用方先 SW_RESTORE。
// SWP_NOACTIVATE 保证不抢别的窗口焦点。
unsafe fn bring_to_front(hwnd: HWND) {
    let _ = unsafe { ShowWindow(hwnd, SW_SHOWNA) };
    let _ = unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOP),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
    };
}

fn embed(window: &tauri::Window) -> Result<(), String> {
    let hwnd = get_hwnd(window)?;
    let host = find_host().ok_or("未找到桌面宿主，嵌入失败")?;
    // 排查交互问题时，日志先把宿主层级说清楚（只打层级名，不打句柄与环境）
    println!(
        "[winembed] 嵌入 {}",
        if prefer_desktop_host() {
            "桌面图标层宿主（实验）"
        } else {
            "壁纸层 WorkerW"
        }
    );
    unsafe {
        let _ = SetParent(hwnd, Some(host));
        // WS_EX_TOOLWINDOW：不进 Alt+Tab / 任务栏
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | (WS_EX_TOOLWINDOW.0 as isize));
        // 与同层兄弟窗口竞争 Z 序，确保自己可见；父在桌面层，仍被所有普通窗口盖住
        bring_to_front(hwnd);
    }
    *EMBED_PARENT.lock().unwrap() = Some(host.0 as isize);
    EMBEDDED.store(true, Ordering::Relaxed);
    Ok(())
}

fn unembed(window: &tauri::Window) -> Result<(), String> {
    let hwnd = get_hwnd(window)?;
    unsafe {
        let _ = SetParent(hwnd, None);
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex & !(WS_EX_TOOLWINDOW.0 as isize));
    }
    EMBEDDED.store(false, Ordering::Relaxed);
    *EMBED_PARENT.lock().unwrap() = None;
    Ok(())
}

// 是否需要唤回：不限嵌入模式（置顶 / 嵌入失败时 Win+D 一样会最小化窗口）；主人主动隐藏时放手
fn should_restore(user_hidden: bool, visible: bool, iconic: bool) -> bool {
    !user_hidden && (!visible || iconic)
}

// 守护每轮调用。不再假设「Win+D 一定改了 WS_VISIBLE」：
// ① 窗口/父链被藏或最小化就立即唤回；② 即使状态看着健康，也按 raise_z 低频把窗口
// 重新提到桌面层 Z 序顶部，兜住 Shell 把桌面压到窗口前面（visible 却看不见）的情况。
// 返回 false 表示唤回后仍不健康，调用方据此退避，避免与 Shell 高频拉锯。
pub fn guard_tick(window: &tauri::WebviewWindow, raise_z: bool) -> bool {
    let Ok(hwnd) = window.hwnd() else { return true };
    let (visible, iconic) = unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            return true;
        }
        (IsWindowVisible(hwnd).as_bool(), IsIconic(hwnd).as_bool())
    };
    // 主人从任务栏/托盘把窗口叫回来了，守护重新接管
    if USER_HIDDEN.load(Ordering::Relaxed) && visible && !iconic {
        USER_HIDDEN.store(false, Ordering::Relaxed);
    }
    let user_hidden = USER_HIDDEN.load(Ordering::Relaxed);
    let need = should_restore(user_hidden, visible, iconic);
    if !need {
        GUARD_LOGGED.store(false, Ordering::Relaxed);
    } else if !GUARD_LOGGED.swap(true, Ordering::Relaxed) {
        eprintln!(
            "[winembed] 守护唤回: visible={visible} iconic={iconic} embedded={} user_hidden={user_hidden}",
            EMBEDDED.load(Ordering::Relaxed)
        );
    }
    let embedded = EMBEDDED.load(Ordering::Relaxed);
    // 未嵌入的普通窗口不做 Z 序提速，否则会一直盖住别的应用
    let raise = raise_z && embedded;
    if (!need && !raise) || user_hidden || mouse_busy() {
        return true;
    }
    unsafe {
        let parent_dead = embedded
            && embed_parent()
                .map(|p| !IsWindow(Some(p)).as_bool())
                .unwrap_or(true);
        if parent_dead
            && let Some(host) = find_host()
        {
            let _ = SetParent(hwnd, Some(host));
            *EMBED_PARENT.lock().unwrap() = Some(host.0 as isize);
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | (WS_EX_TOOLWINDOW.0 as isize));
        }
        // 父被 Win+D 藏了，只 Show 子窗口是看不见的（IsWindowVisible 查整条父链）
        if embedded
            && let Some(p) = embed_parent()
            && IsWindow(Some(p)).as_bool()
            && !IsWindowVisible(p).as_bool()
        {
            let _ = ShowWindow(p, SW_SHOWNA);
        }
        // 顶层窗口被 Win+D 最小化时 SW_SHOWNA 唤不回，必须先 SW_RESTORE
        if iconic {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        if !visible {
            let _ = ShowWindow(hwnd, SW_SHOWNA);
        }
        if raise || need {
            bring_to_front(hwnd);
        }
        IsWindowVisible(hwnd).as_bool() && !IsIconic(hwnd).as_bool()
    }
}

#[tauri::command]
pub fn set_window_mode(window: tauri::Window, topmost: bool) -> Result<(), String> {
    window.set_always_on_top(topmost).map_err(|e| e.to_string())?;
    if topmost {
        unembed(&window)
    } else if std::env::var("GHR_NO_EMBED").is_ok() {
        Ok(()) // 调试开关：跳过嵌入，保持普通窗口
    } else {
        embed(&window)
    }
}

#[cfg(test)]
mod winembed_tests {
    use super::*;

    #[test]
    fn restore_covers_hidden_or_minimized() {
        assert!(should_restore(false, false, false));
        assert!(should_restore(false, true, true));
    }

    #[test]
    fn restore_covers_hidden_and_minimized_combined() {
        assert!(should_restore(false, false, true));
    }

    #[test]
    fn restore_never_fights_user_hidden() {
        assert!(!should_restore(true, false, true));
        assert!(!should_restore(true, false, false));
        assert!(!should_restore(true, true, false));
    }

    #[test]
    fn restore_skips_healthy_window() {
        assert!(!should_restore(false, true, false));
    }

    #[test]
    fn host_defaults_to_interactive_wallpaper_layer() {
        let desktop = HWND(11 as *mut _);
        let workerw = HWND(22 as *mut _);
        assert_eq!(
            choose_host(Some(desktop), Some(workerw), false).map(|h| h.0 as isize),
            Some(22)
        );
    }

    #[test]
    fn experimental_flag_prefers_desktop_icon_layer() {
        let desktop = HWND(11 as *mut _);
        let workerw = HWND(22 as *mut _);
        assert_eq!(
            choose_host(Some(desktop), Some(workerw), true).map(|h| h.0 as isize),
            Some(11)
        );
    }

    #[test]
    fn host_falls_back_when_preferred_layer_missing() {
        let desktop = HWND(11 as *mut _);
        let workerw = HWND(22 as *mut _);
        assert_eq!(
            choose_host(None, Some(workerw), true).map(|h| h.0 as isize),
            Some(22)
        );
        assert_eq!(
            choose_host(Some(desktop), None, false).map(|h| h.0 as isize),
            Some(11)
        );
        assert!(choose_host(None, None, false).is_none());
    }
}
