//! The M1 spike (ADR-001 action item 3, carried into ADR-006).
//!
//! Answers the one question that gates the rest of Phase 2: **can a raw Win32 layered window
//! behave itself as a floating camera bubble?** Everything downstream — M2 capture, M4 camera
//! integration — assumes yes.
//!
//! It deliberately draws a *synthetic* animated pattern rather than a real camera. Every
//! go/no-go criterion below is about window behaviour, not image quality, and mixing in
//! camera enumeration would mean a failure here could be blamed on a webcam driver. The
//! camera arrives in M4, once this is settled.
//!
//! Run it:
//!
//! ```text
//! cargo run -p vrecnu-bubble-win32 --example bubble_spike
//! ```
//!
//! ## What to check, and the thresholds that count as a pass
//!
//! | Check | Pass |
//! |---|---|
//! | Stays above a maximized Chrome, a maximized VS Code, and borderless-fullscreen video | visible in all three |
//! | Clicking the bubble never steals focus (keep typing in another window) | no focus loss |
//! | Dragging the bubble moves it | moves smoothly |
//! | CPU, whole process, at 30 fps | <= 3% of one core |
//! | RSS | <= 15 MB above idle |
//! | Corners are transparent, edge is smooth (no black box, no stair-stepping) | clean circle |
//! | Drag across a 100% and a 150% DPI monitor | no corruption |
//! | Ctrl+C in the console exits cleanly, window disappears | clean exit |
//!
//! Record the results in ADR-006's action item 6.

fn main() {
    #[cfg(not(windows))]
    {
        eprintln!("The bubble spike is Windows-only.");
    }

    #[cfg(windows)]
    {
        run();
    }
}

#[cfg(windows)]
fn run() {
    use std::time::{Duration, Instant};
    use vrecnu_bubble_win32::{render_bubble, BubbleLayout, LayeredBubble, SquareFrame};
    use vrecnu_types::BubbleSpec;

    let spec = BubbleSpec {
        diameter: 240,
        position: (120, 120),
        // Click-through OFF for the spike: the window has to be draggable to test dragging.
        click_through: false,
    };
    let layout = BubbleLayout::from_spec(spec);

    println!("vrecnu bubble spike (M1)");
    println!(
        "  window {}x{} at ({}, {}), circle {} px",
        layout.window.width,
        layout.window.height,
        layout.window.x,
        layout.window.y,
        layout.circle_diameter
    );
    println!("  drag it, put a fullscreen window behind it, then Ctrl+C to exit.");
    println!();

    let bubble = match LayeredBubble::spawn(spec) {
        Ok(bubble) => bubble,
        Err(err) => {
            eprintln!("could not create the bubble window: {err}");
            std::process::exit(1);
        }
    };

    let frame_interval = Duration::from_nanos(1_000_000_000 / 30);
    let started = Instant::now();
    let mut frames: u64 = 0;
    let mut next = Instant::now();

    // Ctrl+C sets this, so teardown goes through Drop rather than killing the process and
    // leaving a topmost window behind.
    let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let signal = std::sync::Arc::clone(&running);
    let _ = ctrl_c_handler(move || signal.store(false, std::sync::atomic::Ordering::Relaxed));

    while running.load(std::sync::atomic::Ordering::Relaxed) && bubble.is_running() {
        let elapsed = started.elapsed().as_secs_f32();
        let frame = animated_frame(layout.circle_diameter, elapsed);
        let surface = render_bubble(&layout, &frame);

        if let Err(err) = bubble.present(&surface) {
            eprintln!("present failed: {err}");
            break;
        }
        frames += 1;

        if frames % 150 == 0 {
            println!(
                "  {frames} frames, {:.1} fps average",
                frames as f32 / started.elapsed().as_secs_f32()
            );
        }

        // Fixed deadline rather than sleep-per-iteration, so render time does not accumulate
        // into a drifting frame rate.
        next += frame_interval;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else {
            next = now;
        }
    }

    println!();
    println!(
        "stopped after {frames} frames in {:.1}s",
        started.elapsed().as_secs_f32()
    );

    /// A moving colour field, so motion and tearing are visible.
    fn animated_frame(side: u32, t: f32) -> SquareFrame {
        let mut bgra = vec![0u8; (side as usize) * (side as usize) * 4];
        let sweep = (t * 0.7).sin() * 0.5 + 0.5;
        for y in 0..side {
            for x in 0..side {
                let index = ((y * side + x) * 4) as usize;
                let fx = x as f32 / side as f32;
                let fy = y as f32 / side as f32;
                bgra[index] = (255.0 * (1.0 - fx) * sweep) as u8;
                bgra[index + 1] = (255.0 * fy) as u8;
                bgra[index + 2] = (255.0 * fx * (1.0 - sweep)) as u8;
                bgra[index + 3] = 255;
            }
        }
        // A rotating marker makes dropped or duplicated frames obvious to the naked eye.
        let angle = t * 2.0;
        let cx = (side as f32 / 2.0 + angle.cos() * side as f32 * 0.3) as u32;
        let cy = (side as f32 / 2.0 + angle.sin() * side as f32 * 0.3) as u32;
        for dy in 0..12u32 {
            for dx in 0..12u32 {
                let (px, py) = (cx.saturating_add(dx), cy.saturating_add(dy));
                if px < side && py < side {
                    let index = ((py * side + px) * 4) as usize;
                    bgra[index..index + 4].copy_from_slice(&[255, 255, 255, 255]);
                }
            }
        }
        SquareFrame { side, bgra }
    }
}

/// Minimal Ctrl+C hook so the spike needs no extra dependency.
#[cfg(windows)]
fn ctrl_c_handler<F: FnMut() + Send + 'static>(handler: F) -> std::io::Result<()> {
    use std::sync::Mutex;
    static HANDLER: Mutex<Option<Box<dyn FnMut() + Send>>> = Mutex::new(None);

    if let Ok(mut slot) = HANDLER.lock() {
        *slot = Some(Box::new(handler));
    }

    unsafe extern "system" fn on_ctrl(_kind: u32) -> windows::core::BOOL {
        if let Ok(mut slot) = HANDLER.lock() {
            if let Some(callback) = slot.as_mut() {
                callback();
            }
        }
        windows::core::BOOL(1)
    }

    // SAFETY: registering a console control handler with a valid `extern "system"` function
    // that lives for the life of the process.
    unsafe {
        windows::Win32::System::Console::SetConsoleCtrlHandler(Some(on_ctrl), true)
            .map_err(|err| std::io::Error::other(err.to_string()))?;
    }
    Ok(())
}
