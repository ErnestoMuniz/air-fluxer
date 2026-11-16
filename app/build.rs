//! macOS: WebRTC ships its Objective-C helpers as categories in a static library. Without `-ObjC`
//! the linker drops the archive members that only define categories, and creating a
//! PeerConnectionFactory aborts with `+[NSString stringForStdString:]: unrecognized selector`
//! (WebRTC's H.264 encoder factory calls it). Other platforms have no such categories.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-ObjC");
    }
}
