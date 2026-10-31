use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use windows::core::{w, BOOL, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowExW, FindWindowW, GetClassNameW, GetWindowLongPtrW, IsIconic,
    IsWindowVisible, SendMessageW, SetParent, SetWindowLongPtrW, ShowWindow, GWL_EXSTYLE,
    SW_RESTORE, WS_EX_TOOLWINDOW,
};

// 嵌入目标 WorkerW 的句柄，枚举回调里写，枚举结束后读
static WORKERW: Mutex<Option<isize>> = Mutex::new(None);
static DEFVIEW_FOUND: AtomicBool = AtomicBool::new(false);
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
        // 桌面图标层（SHELLDLL_DefView）所在窗口之后出现的第一个 WorkerW 就是嵌入目标
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

fn embed(window: &tauri::Window) -> Result<(), String> {
    let hwnd = get_hwnd(window)?;
    let workerw = find_workerw().ok_or("未找到 WorkerW，嵌入失败")?;
    unsafe { let _ = SetParent(hwnd, Some(workerw)); };
    // WS_EX_TOOLWINDOW：不进 Alt+Tab / 任务栏
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | (WS_EX_TOOLWINDOW.0 as isize));
    }
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
    Ok(())
}

// Win+D 会把贴桌面的嵌入窗口最小化/隐藏，而它不在任务栏，Win+D 再按一次也唤不回来，
// 只能自己定时查：嵌入模式下若被藏了就 SW_RESTORE。用户主动隐藏的（USER_HIDDEN）不管。
pub fn restore_if_hidden_by_win_d(window: &tauri::WebviewWindow) {
    if !EMBEDDED.load(Ordering::Relaxed) || USER_HIDDEN.load(Ordering::Relaxed) {
        return;
    }
    let Ok(hwnd) = window.hwnd() else { return };
    unsafe {
        if IsWindowVisible(hwnd).as_bool() && !IsIconic(hwnd).as_bool() {
            return;
        }
        let _ = ShowWindow(hwnd, SW_RESTORE);
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
