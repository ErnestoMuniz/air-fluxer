//! macOS: WebRTC ships its Objective-C helpers as categories in a static library. Without `-ObjC`
//! the linker drops the archive members that only define categories, and creating a
//! PeerConnectionFactory aborts with `+[NSString stringForStdString:]: unrecognized selector`
//! (WebRTC's H.264 encoder factory calls it). Other platforms have no such categories.
//!
//! Linux: screen sharing pulls in WebRTC's desktop capturer, whose xdg-desktop-portal code calls
//! GLib/GIO. webrtc-sys only needs their headers to compile, so the app links them itself.
fn main() {
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("macos") => println!("cargo:rustc-link-arg=-ObjC"),
        Ok("linux") => for lib in ["gio-2.0", "gobject-2.0", "glib-2.0"] { println!("cargo:rustc-link-lib=dylib={lib}") },
        _ => {}
    }
}
