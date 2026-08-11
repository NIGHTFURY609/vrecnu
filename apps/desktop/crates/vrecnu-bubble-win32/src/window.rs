//! The layered window itself.
//!
//! Runs on its own thread with its own message loop. Win32 message loops are per-thread by
//! design, so this coexists with Tauri's `tao` loop without either knowing about the other —
//! which is exactly why the bubble is raw Win32 rather than a second `winit` window.
//!
//! All `unsafe` here is Win32 FFI. Each block documents the invariant it relies on.

#![allow(unsafe_code)]

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use vrecnu_types::{BubbleSpec, Rect};
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
    HBITMAP,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    PostThreadMessageW, RegisterClassExW, SetWindowPos, ShowWindow, TranslateMessage,
    UpdateLayeredWindow, HTCAPTION, HWND_TOPMOST, MSG, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SW_SHOWNOACTIVATE, ULW_ALPHA, WM_APP, WM_NCHITTEST, WM_QUIT, WNDCLASSEXW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::geometry::BubbleLayout;
use crate::BubbleError;

/// Posted to the window thread when a new surface is waiting.
const WM_BUBBLE_PRESENT: u32 = WM_APP + 1;
/// Posted to the window thread to tear the window down.
const WM_BUBBLE_CLOSE: u32 = WM_APP + 2;

/// Shared state between the presenting thread and the window thread.
#[derive(Debug, Default)]
struct Shared {
    /// The next surface to present, premultiplied BGRA.
    pending: Mutex<Option<Vec<u8>>>,
    width: AtomicU32,
    height: AtomicU32,
    x: AtomicI32,
    y: AtomicI32,
    click_through: AtomicBool,
    running: AtomicBool,
}

/// An `HWND` that may cross threads.
///
/// # Safety
/// `HWND` is an opaque kernel handle, not a pointer into this process's heap. The only calls
/// made on it from another thread are `PostMessageW`/`PostThreadMessageW`, which Win32
/// documents as thread-safe. Every call that touches window *state* happens on the window
/// thread inside the message loop.
#[derive(Clone, Copy)]
struct SendHwnd(HWND);

// SAFETY: see the doc comment above — handle-only, and cross-thread use is limited to the
// thread-safe Post* family.
unsafe impl Send for SendHwnd {}
// SAFETY: as above; the handle is immutable once created.
unsafe impl Sync for SendHwnd {}

/// A live bubble window.
pub struct LayeredBubble {
    shared: Arc<Shared>,
    thread_id: u32,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl std::fmt::Debug for LayeredBubble {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LayeredBubble")
            .field("thread_id", &self.thread_id)
            .field("running", &self.shared.running.load(Ordering::Relaxed))
            .finish()
    }
}

impl LayeredBubble {
    /// Creates the window on a dedicated thread and returns once it is ready.
    ///
    /// # Errors
    /// Returns [`BubbleError::WindowCreation`] if the class or window cannot be created.
    pub fn spawn(spec: BubbleSpec) -> Result<Self, BubbleError> {
        let layout = BubbleLayout::from_spec(spec);
        let shared = Arc::new(Shared::default());
        shared.width.store(layout.window.width, Ordering::Relaxed);
        shared.height.store(layout.window.height, Ordering::Relaxed);
        shared.x.store(layout.window.x, Ordering::Relaxed);
        shared.y.store(layout.window.y, Ordering::Relaxed);
        shared
            .click_through
            .store(spec.click_through, Ordering::Relaxed);

        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<u32, String>>();
        let thread_shared = Arc::clone(&shared);

        let handle = std::thread::Builder::new()
            .name("vrecnu-bubble".into())
            .spawn(move || window_thread(&thread_shared, &ready_tx))
            .map_err(|err| BubbleError::WindowCreation(err.to_string()))?;

        match ready_rx.recv() {
            Ok(Ok(thread_id)) => {
                shared.running.store(true, Ordering::Release);
                Ok(Self {
                    shared,
                    thread_id,
                    handle: Some(handle),
                })
            }
            Ok(Err(message)) => Err(BubbleError::WindowCreation(message)),
            Err(_) => Err(BubbleError::WindowCreation(
                "the bubble window thread exited before it was ready".into(),
            )),
        }
    }

    /// Queues a surface and wakes the window thread.
    ///
    /// Called from the camera thread. It stores and posts rather than drawing inline, because
    /// GDI calls must happen on the thread that owns the window.
    ///
    /// # Errors
    /// Returns [`BubbleError::ThreadGone`] if the window thread has stopped.
    pub fn present(&self, surface: Vec<u8>) -> Result<(), BubbleError> {
        if !self.shared.running.load(Ordering::Acquire) {
            return Err(BubbleError::ThreadGone);
        }
        // Replace rather than queue: if the window thread has not drawn the previous frame
        // yet, the newer one is strictly better and the old one is worthless.
        *self
            .shared
            .pending
            .lock()
            .map_err(|_| BubbleError::ThreadGone)? = Some(surface);

        // SAFETY: `thread_id` came from `GetCurrentThreadId` on a thread that was running a
        // message loop, and PostThreadMessageW is documented thread-safe. A failure here just
        // means the thread has gone, which the next `present` reports.
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_BUBBLE_PRESENT, WPARAM(0), LPARAM(0));
        }
        Ok(())
    }

    /// Current window rectangle, which the user may have dragged.
    #[must_use]
    pub fn geometry(&self) -> Rect {
        Rect {
            x: self.shared.x.load(Ordering::Relaxed),
            y: self.shared.y.load(Ordering::Relaxed),
            width: self.shared.width.load(Ordering::Relaxed),
            height: self.shared.height.load(Ordering::Relaxed),
        }
    }

    /// Whether the window thread is still alive.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.shared.running.load(Ordering::Acquire)
    }

    /// Closes the window and joins its thread.
    pub fn close(&mut self) {
        if !self.shared.running.swap(false, Ordering::AcqRel) {
            return;
        }
        // SAFETY: thread-safe Post* call; see `present`.
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_BUBBLE_CLOSE, WPARAM(0), LPARAM(0));
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for LayeredBubble {
    fn drop(&mut self) {
        self.close();
    }
}

/// Body of the window thread: register, create, pump, clean up.
fn window_thread(shared: &Arc<Shared>, ready: &std::sync::mpsc::Sender<Result<u32, String>>) {
    // SAFETY: every call below is a straightforward Win32 FFI call with valid arguments; the
    // window and DC handles created here never leave this thread except as `SendHwnd`.
    unsafe {
        let instance = match GetModuleHandleW(None) {
            Ok(handle) => handle,
            Err(err) => {
                let _ = ready.send(Err(format!("GetModuleHandleW failed: {err}")));
                return;
            }
        };

        let class_name = w!("vrecnu_bubble_window");
        let class = WNDCLASSEXW {
            cbSize: u32::try_from(std::mem::size_of::<WNDCLASSEXW>()).unwrap_or(0),
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: class_name,
            ..Default::default()
        };
        // Re-registering an existing class returns 0; that is fine and expected if a previous
        // bubble ran in this process, so the return value is deliberately not fatal.
        let _ = RegisterClassExW(&class);

        let width = shared.width.load(Ordering::Relaxed) as i32;
        let height = shared.height.load(Ordering::Relaxed) as i32;

        let mut ex_style = WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
        if shared.click_through.load(Ordering::Relaxed) {
            ex_style |= WS_EX_TRANSPARENT;
        }

        let hwnd = match CreateWindowExW(
            ex_style,
            class_name,
            w!("vrecnu"),
            WS_POPUP,
            shared.x.load(Ordering::Relaxed),
            shared.y.load(Ordering::Relaxed),
            width,
            height,
            None,
            None,
            Some(instance.into()),
            None,
        ) {
            Ok(hwnd) => hwnd,
            Err(err) => {
                let _ = ready.send(Err(format!("CreateWindowExW failed: {err}")));
                return;
            }
        };

        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );

        let thread_id = GetCurrentThreadId();
        if ready.send(Ok(thread_id)).is_err() {
            let _ = DestroyWindow(hwnd);
            return;
        }

        let window = SendHwnd(hwnd);
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            match message.message {
                WM_BUBBLE_PRESENT => {
                    let surface = shared.pending.lock().ok().and_then(|mut slot| slot.take());
                    if let Some(surface) = surface {
                        present_surface(window, shared, &surface);
                    }
                }
                WM_BUBBLE_CLOSE | WM_QUIT => break,
                _ => {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        }

        let _ = DestroyWindow(hwnd);
        shared.running.store(false, Ordering::Release);
    }
}

/// Pushes one premultiplied-BGRA surface to the window.
///
/// # Safety
/// Caller must be the window thread. `surface` must be exactly
/// `width * height * 4` bytes of premultiplied BGRA.
unsafe fn present_surface(window: SendHwnd, shared: &Arc<Shared>, surface: &[u8]) {
    let width = shared.width.load(Ordering::Relaxed) as i32;
    let height = shared.height.load(Ordering::Relaxed) as i32;
    if surface.len() < (width * height * 4) as usize {
        return;
    }

    unsafe {
        let screen_dc = GetDC(None);
        let mem_dc = CreateCompatibleDC(Some(screen_dc));

        // Negative height requests a top-down DIB, matching our buffer's row order. Omitting
        // it renders the bubble upside down, which is a classic GDI mistake.
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: u32::try_from(std::mem::size_of::<BITMAPINFOHEADER>()).unwrap_or(0),
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bitmap: HBITMAP =
            match CreateDIBSection(Some(mem_dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
                Ok(bitmap) if !bits.is_null() => bitmap,
                _ => {
                    let _ = DeleteDC(mem_dc);
                    ReleaseDC(None, screen_dc);
                    return;
                }
            };
        info.bmiHeader.biSizeImage = 0;

        std::ptr::copy_nonoverlapping(
            surface.as_ptr(),
            bits.cast::<u8>(),
            (width * height * 4) as usize,
        );

        let old = SelectObject(mem_dc, bitmap.into());

        let size = windows::Win32::Foundation::SIZE {
            cx: width,
            cy: height,
        };
        let src_point = POINT { x: 0, y: 0 };
        // Not `mut`: `UpdateLayeredWindow` takes this by const pointer. Passing `&mut` compiles
        // through the coercion and reads as though Windows moves the window for us, which it
        // does not — the position is ours to track.
        let dst_point = POINT {
            x: shared.x.load(Ordering::Relaxed),
            y: shared.y.load(Ordering::Relaxed),
        };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };

        let _ = UpdateLayeredWindow(
            window.0,
            Some(screen_dc),
            Some(&dst_point),
            Some(&size),
            Some(mem_dc),
            Some(&src_point),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        );

        SelectObject(mem_dc, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(mem_dc);
        ReleaseDC(None, screen_dc);
    }
}

/// Window procedure.
///
/// # Safety
/// Called by Win32 on the window thread with valid handles.
unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        // Reporting the whole surface as the title bar is what makes the bubble draggable
        // in three lines instead of tracking mouse-down/move/up by hand.
        WM_NCHITTEST => LRESULT(HTCAPTION as isize),
        _ => {
            // SAFETY: forwarding to the default handler with the arguments Win32 supplied.
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
    }
}
