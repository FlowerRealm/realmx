fn main() {
    let mut res = winres::WindowsResource::new();
    res.set_manifest_file("realmx-windows-sandbox-setup.manifest");
    let _ = res.compile();
}
