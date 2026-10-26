fn main() {
    // Embed the app icon into NumberLock.exe.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico").set("ProductName", "NumberLock").set("FileDescription", "NumberLock");
        res.compile().expect("windows resources");
    }
}
