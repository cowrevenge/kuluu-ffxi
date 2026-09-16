//! Every registered FFXI install, one `NAME<TAB>ROOT` line each; `--roots`
//! prints the roots alone, which is how `scripts/checks.sh install` finds
//! them without a second copy of the registry convention.
fn main() {
    let roots_only = std::env::args().any(|a| a == "--roots");
    for i in ffxi_dat::install::list() {
        if roots_only {
            println!("{}", i.path.display());
        } else {
            println!("{}\t{}", i.name, i.path.display());
        }
    }
}
