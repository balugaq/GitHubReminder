use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use windows::core::{w, BOOL, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowExW, FindWindowW, GetClassNameW, GetWindowLongPtrW, IsIconic, IsWindow,
    IsWindowVisible, SendMessageW, SetParent, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    GWL_EXSTYLE, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_SHOWNA,
    WS_EX_TOOLWINDOW,
};

// 壁纸层 WorkerW 的句柄（旧嵌入方式 / 回退用），枚举回调里写，枚举结束后读
static WORKERW: Mutex<Option<isize>> = Mutex::new(None);
static DEFVIEW_FOUND: AtomicBool = AtomicBool::new(false);
// 桌面图标层 SHELLDLL_DefView 的直接宿主：Progman 或挂着 DefView 的那个 WorkerW。
// Win+D /「显示桌面」会跳过这一层，挂到它下面才能像 Win7 小组件一样一直留在桌面上。
static DESKTOP_HOST: Mutex<Option<isize>> = Mutex::new(None);
// 本次实际 SetParent 的父句柄，守护协程据此判断父链是否被 Explorer 重建
static EMBED_PARENT: Mutex<Option<isize>> = Mutex::new(None);
// 当前是否处于嵌入模式；Win+D 守护协程据此决定要不要唤回
static EMBEDDED: AtomicBool = AtomicBool::new(false);
// 用户主动隐藏（托盘开关/点关闭）时置位，守护协程见到就放手，否则会和主人抢窗口
pub static USER_HIDDEN: AtomicBool = AtomicBool::new(false);

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
// 不用 0x052C 分裂出的壁纸层：那层在 Win+D 时会跟桌面一起被隐藏，窗口作为子窗口跟着消失。
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

// 桌面图标层宿主优先；找不到（极少数 Shell 布局异常）才退回壁纸层
fn choose_host(desktop: Option<HWND>, workerw: Option<HWND>) -> Option<HWND> {
    desktop.or(workerw)
}

// 复用的展示动作：子窗口没有 minimized/maximized 概念，SW_RESTORE 对它无效，
// 要用 SW_SHOWNA + SWP_SHOWWINDOW；SWP_NOACTIVATE 保证不抢别的窗口焦点。
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
    // GHR_WALLPAPER_LAYER：万一新宿主在某个系统上表现不对，设这个环境变量挂回旧的壁纸层
    let force_wallpaper = std::env::var("GHR_WALLPAPER_LAYER").is_ok();
    let desktop = if force_wallpaper {
        None
    } else {
        find_desktop_host()
    };
    // 只有首选落空才发 0x052C 分裂壁纸层，避免每轮嵌入都去动 Explorer
    let workerw = if desktop.is_none() {
        find_workerw()
    } else {
        None
    };
    let host = choose_host(desktop, workerw).ok_or("未找到桌面宿主，嵌入失败")?;
    unsafe {
        let _ = SetParent(hwnd, Some(host));
        // WS_EX_TOOLWINDOW：不进 Alt+Tab / 任务栏
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | (WS_EX_TOOLWINDOW.0 as isize));
        // 与 DefView 是兄弟，放到 Z 序顶部 = 图标之上；父在桌面层，仍被所有普通窗口盖住
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

// 是否需要动手唤回：只管贴桌面嵌入模式；主人主动隐藏时绝不插手
fn should_restore(embedded: bool, user_hidden: bool, visible: bool, iconic: bool) -> bool {
    embedded && !user_hidden && (!visible || iconic)
}

// 免疫 Win+D 的兜底。主防线是 embed() 挂到 SHELLDLL_DefView 的宿主（Shell 跳过它）；
// 万一某些 Shell 版本仍把桌面层藏了，这里负责把父窗口和子窗口一起显示回来，
// 顺带在 Explorer 重建导致父句柄失效时重新嵌入。
pub fn restore_if_hidden_by_win_d(window: &tauri::WebviewWindow) {
    let Ok(hwnd) = window.hwnd() else { return };
    let (visible, iconic) = unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            return;
        }
        (IsWindowVisible(hwnd).as_bool(), IsIconic(hwnd).as_bool())
    };
    if !should_restore(
        EMBEDDED.load(Ordering::Relaxed),
        USER_HIDDEN.load(Ordering::Relaxed),
        visible,
        iconic,
    ) {
        return;
    }
    unsafe {
        // Explorer 重启会让 Progman / WorkerW 句柄失效，重新挂到当前桌面宿主
        let parent_dead = embed_parent()
            .map(|p| !IsWindow(Some(p)).as_bool())
            .unwrap_or(true);
        if parent_dead
            && let Some(host) = find_desktop_host().or_else(find_workerw)
        {
            let _ = SetParent(hwnd, Some(host));
            *EMBED_PARENT.lock().unwrap() = Some(host.0 as isize);
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | (WS_EX_TOOLWINDOW.0 as isize));
        }
        // 父被 Win+D 藏了，只 Show 子窗口是看不见的（IsWindowVisible 查整条父链）
        if let Some(p) = embed_parent()
            && IsWindow(Some(p)).as_bool()
            && !IsWindowVisible(p).as_bool()
        {
            let _ = ShowWindow(p, SW_SHOWNA);
        }
        bring_to_front(hwnd);
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
    fn restore_only_in_embedded_mode() {
        assert!(!should_restore(false, false, false, false));
        assert!(should_restore(true, false, false, false));
    }

    #[test]
    fn restore_never_fights_user_hidden() {
        assert!(!should_restore(true, true, false, true));
        assert!(!should_restore(true, true, true, false));
    }

    #[test]
    fn restore_skips_healthy_window() {
        assert!(!should_restore(true, false, true, false));
    }

    #[test]
    fn restore_catches_hidden_or_minimized() {
        assert!(should_restore(true, false, false, false));
        assert!(should_restore(true, false, true, true));
    }

    #[test]
    fn host_prefers_desktop_icon_layer() {
        let desktop = HWND(11 as *mut _);
        let workerw = HWND(22 as *mut _);
        assert_eq!(
            choose_host(Some(desktop), Some(workerw)).map(|h| h.0 as isize),
            Some(11)
        );
        assert_eq!(
            choose_host(None, Some(workerw)).map(|h| h.0 as isize),
            Some(22)
        );
        assert!(choose_host(None, None).is_none());
    }
}
