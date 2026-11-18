//! macOS screen capture through CGWindowListCreateImage, which works from a background thread
//! (unlike ScreenCaptureKit's picker) and needs only the Screen Recording permission.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

type CGImageRef = *mut c_void;
type CGDataProviderRef = *mut c_void;
type CFDataRef = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGPoint { x: f64, y: f64 }
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGSize { width: f64, height: f64 }
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGRect { origin: CGPoint, size: CGSize }

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGMainDisplayID() -> u32;
    fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
    fn CGDisplayBounds(display: u32) -> CGRect;
    fn CGWindowListCreateImage(rect: CGRect, option: u32, window: u32, image_option: u32) -> CGImageRef;
    fn CGImageGetWidth(image: CGImageRef) -> usize;
    fn CGImageGetHeight(image: CGImageRef) -> usize;
    fn CGImageGetBytesPerRow(image: CGImageRef) -> usize;
    fn CGImageGetDataProvider(image: CGImageRef) -> CGDataProviderRef;
    fn CGDataProviderCopyData(provider: CGDataProviderRef) -> CFDataRef;
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDataGetBytePtr(data: CFDataRef) -> *const u8;
    fn CFDataGetLength(data: CFDataRef) -> isize;
    fn CFRelease(object: *const c_void);
}

/// Asks for Screen Recording; false means the user denied it (or it needs an app restart).
pub fn permission() -> bool { unsafe { CGPreflightScreenCaptureAccess() || CGRequestScreenCaptureAccess() } }

/// The displays the system offers, as (id, "Screen N (WxH)").
pub fn displays() -> Vec<(u32, String)> {
    let mut ids = [0u32; 16];
    let mut count = 0u32;
    let ok = unsafe { CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) } == 0;
    if !ok { return vec![] }
    ids[..count as usize].iter().enumerate().map(|(i, id)| {
        let b = unsafe { CGDisplayBounds(*id) };
        (*id, format!("Tela {} ({}x{})", i + 1, b.size.width as u32, b.size.height as u32))
    }).collect()
}

pub fn main_display() -> u32 { unsafe { CGMainDisplayID() } }

/// One BGRA frame and its layout.
pub struct Frame { pub data: Vec<u8>, pub width: usize, pub height: usize, pub stride: usize }

pub fn grab(display: u32) -> Option<Frame> {
    unsafe {
        let rect = CGDisplayBounds(display);
        // On-screen windows only, no desktop elements, whole display.
        let image = CGWindowListCreateImage(rect, 1, 0, 0);
        if image.is_null() { return grab_cli(display) }
        let (width, height, stride) = (CGImageGetWidth(image), CGImageGetHeight(image), CGImageGetBytesPerRow(image));
        let provider = CGImageGetDataProvider(image);
        if provider.is_null() { CFRelease(image as *const c_void); return None }
        let data = CGDataProviderCopyData(provider);
        if data.is_null() { CFRelease(image as *const c_void); return None }
        let ptr = CFDataGetBytePtr(data);
        let len = CFDataGetLength(data).max(0) as usize;
        let out = if ptr.is_null() || width == 0 || height == 0 || len < stride * height {
            None
        } else {
            Some(Frame { data: std::slice::from_raw_parts(ptr, stride * height).to_vec(), width, height, stride })
        };
        CFRelease(data as *const c_void);
        CFRelease(image as *const c_void);
        out
    }
}

/// A thread that captures `display` at `fps` until the returned flag is set.
pub fn start(display: u32, fps: u32, mut on_frame: impl FnMut(Frame) + Send + 'static) -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    std::thread::Builder::new().name("fluxer-screen".into()).spawn(move || {
        let gap = std::time::Duration::from_millis((1000 / fps.max(1) as u64).max(1));
        while !flag.load(Ordering::Relaxed) {
            let started = std::time::Instant::now();
            if let Some(frame) = grab(display) { on_frame(frame) }
            if let Some(rest) = gap.checked_sub(started.elapsed()) { std::thread::sleep(rest) }
        }
    }).ok();
    stop
}

/// Fallback when CGWindowList refuses to hand over an image (macOS 26+/permission quirks): ask the
/// system's `screencapture` for one frame. Slower, but it proves and keeps the video path alive.
fn grab_cli(display: u32) -> Option<Frame> {
    let path = std::env::temp_dir().join(format!("airfluxer-{display}.png"));
    let ok = std::process::Command::new("/usr/sbin/screencapture")
        .args(["-x", "-D", &display.to_string(), "-t", "png"]).arg(&path)
        .status().ok()?.success();
    if !ok { return None }
    let image = image::open(&path).ok()?.to_rgba8();
    let (w, h) = image.dimensions();
    // BGRA, the byte order the I420 converter expects from CGWindowList.
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for p in image.pixels() { let [r, g, b, a] = p.0; data.extend_from_slice(&[b, g, r, a]); }
    Some(Frame { data, width: w as usize, height: h as usize, stride: (w * 4) as usize })
}
