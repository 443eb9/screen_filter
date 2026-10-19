use std::{cell::Cell, mem::size_of};

use windows::{
    Win32::{
        Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{HiDpi::*, WindowsAndMessaging::*},
    },
    core::{Error, Result, w},
};

pub struct WindowClass {
    instance: HINSTANCE,
    previous_dpi: DPI_AWARENESS_CONTEXT,
}

impl WindowClass {
    pub fn new() -> Result<Self> {
        unsafe {
            let instance = HINSTANCE(GetModuleHandleW(None)?.0);
            let previous_dpi =
                SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            if previous_dpi.0.is_null() {
                return Err(Error::from_win32());
            }
            let class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                lpszClassName: w!("ScreenFilterOverlay"),
                ..Default::default()
            };
            if RegisterClassExW(&class) == 0 {
                let error = Error::from_win32();
                SetThreadDpiAwarenessContext(previous_dpi);
                return Err(error);
            }
            Ok(Self {
                instance,
                previous_dpi,
            })
        }
    }

    pub fn create(&self, rect: RECT) -> Result<Overlay> {
        let events = Box::new(WindowEvents::default());
        unsafe {
            let handle = CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_TOPMOST
                    | WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE
                    | WS_EX_NOREDIRECTIONBITMAP,
                w!("ScreenFilterOverlay"),
                w!("Screen Filter"),
                WS_POPUP,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                None,
                None,
                Some(self.instance),
                Some((&*events as *const WindowEvents).cast()),
            )?;
            let overlay = Overlay {
                handle,
                events,
                visible: false,
            };
            SetLayeredWindowAttributes(handle, COLORREF(0), 255, LWA_ALPHA)?;
            overlay.set_frozen(false)?;
            Ok(overlay)
        }
    }

    pub fn process_messages(&self) -> bool {
        unsafe {
            let mut message = MSG::default();
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                if message.message == WM_QUIT {
                    return false;
                }
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        true
    }
}

impl Drop for WindowClass {
    fn drop(&mut self) {
        unsafe {
            let _ = UnregisterClassW(w!("ScreenFilterOverlay"), Some(self.instance));
            SetThreadDpiAwarenessContext(self.previous_dpi);
        }
    }
}

#[derive(Default)]
struct WindowEvents {
    changed: Cell<bool>,
    closed: Cell<bool>,
}

pub struct Overlay {
    pub handle: HWND,
    events: Box<WindowEvents>,
    visible: bool,
}

impl Overlay {
    pub fn show(&mut self, visible: bool) {
        if self.visible != visible {
            unsafe {
                let _ = ShowWindow(
                    self.handle,
                    if visible { SW_SHOWNOACTIVATE } else { SW_HIDE },
                );
            }
            self.visible = visible;
        }
    }

    pub fn set_frozen(&self, frozen: bool) -> Result<()> {
        unsafe {
            let affinity = if frozen {
                WDA_NONE
            } else {
                WDA_EXCLUDEFROMCAPTURE
            };
            SetWindowDisplayAffinity(self.handle, affinity)?;
            let mut actual = 0;
            GetWindowDisplayAffinity(self.handle, &mut actual)?;
            if actual != affinity.0 {
                return Err(Error::new(
                    windows::Win32::Foundation::E_FAIL,
                    "Window capture exclusion verification failed",
                ));
            }
        }
        Ok(())
    }

    pub fn changed(&self) -> bool {
        self.events.changed.replace(false)
    }

    pub fn closed(&self) -> bool {
        self.events.closed.get()
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.handle);
        }
    }
}

unsafe extern "system" fn window_proc(
    handle: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        if message == WM_NCCREATE {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(handle, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let events = GetWindowLongPtrW(handle, GWLP_USERDATA) as *const WindowEvents;
        if let Some(events) = events.as_ref() {
            match message {
                WM_DISPLAYCHANGE | WM_SETTINGCHANGE | WM_DPICHANGED => events.changed.set(true),
                WM_CLOSE => {
                    events.closed.set(true);
                    return LRESULT(0);
                }
                WM_NCDESTROY => {
                    SetWindowLongPtrW(handle, GWLP_USERDATA, 0);
                }
                WM_NCHITTEST => return LRESULT(HTTRANSPARENT as isize),
                _ => {}
            }
        }
        DefWindowProcW(handle, message, wparam, lparam)
    }
}
