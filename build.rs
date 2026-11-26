fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    winresource::WindowsResource::new()
        .set_icon("assets/icon.ico")
        .set("ProductName", "VRCLT")
        .set("FileDescription", "VRCLT Live Translator")
        .compile()
        .expect("embed Windows icon");
}
