fn main() {
    // On Windows, HEIC support links libheif statically (vcpkg
    // x64-windows-static-md), which pulls in x265. x265's threadpool code
    // calls the Win32 registry API (RegOpenKeyExA/RegQueryValueExA/RegCloseKey)
    // from advapi32, which static linking does not pull in transitively — so
    // link it explicitly to resolve those symbols.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" && std::env::var("CARGO_FEATURE_HEIC").is_ok() {
        println!("cargo:rustc-link-lib=advapi32");
    }
}
