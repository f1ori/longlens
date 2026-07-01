fn main() {
    println!("cargo:rerun-if-changed=src/rdp/freerdp_bridge.c");
    println!("cargo:rerun-if-changed=src/rdp/freerdp_bridge.h");

    let freerdp = pkg_config::Config::new()
        .atleast_version("3.27.1")
        .cargo_metadata(false)
        .probe("freerdp-client3")
        .expect("FreeRDP client development files >= 3.27.1 are required");

    let mut build = cc::Build::new();
    build
        .file("src/rdp/freerdp_bridge.c")
        .warnings(true)
        .extra_warnings(true)
        .flag_if_supported("-Wno-deprecated-declarations")
        .flag_if_supported("-std=c11");
    for path in freerdp.include_paths {
        build.include(path);
    }
    build.compile("longlens-freerdp");

    for path in freerdp.link_paths {
        println!("cargo:rustc-link-search=native={}", path.display());
    }
    for library in freerdp.libs {
        println!("cargo:rustc-link-lib={library}");
    }
}
