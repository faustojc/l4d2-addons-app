fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    // Icon + "L4D2 Addons Manager" name in Explorer, Task Manager and the taskbar.
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico")
            .set("ProductName", "L4D2 Addons Manager")
            .set("FileDescription", "L4D2 Addons Manager")
            .set("OriginalFilename", "l4d2-addons-manager.exe");
        res.compile().expect("failed to embed Windows resources");
    }
}
