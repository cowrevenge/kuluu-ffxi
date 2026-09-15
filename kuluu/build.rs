fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/branding/kuluu.rc");
    println!("cargo:rerun-if-changed=assets/branding/kuluu.ico");
    embed_resource::compile_for("assets/branding/kuluu.rc", ["kuluu"], embed_resource::NONE)
        .manifest_required()
        .expect("compile the Kuluu Windows icon resource");

    // The launcher footer stamps the build so a bug report identifies its
    // artifact. Cargo exposes the resolved target triple to build scripts only;
    // there is no `env!` for it in the crate. It has to be the *compile* target,
    // not `std::env::consts::ARCH`/`OS`, because the Steam Deck path
    // cross-builds x86_64-unknown-linux-gnu from an Apple Silicon host.
    let target = std::env::var("TARGET").expect("cargo sets TARGET for build scripts");
    println!("cargo:rustc-env=KULUU_TARGET_TRIPLE={target}");
}
