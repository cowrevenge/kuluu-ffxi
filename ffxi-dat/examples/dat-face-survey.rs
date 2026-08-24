//! Survey every PC face DAT (race 1..=8, face 0..=31): does the file resolve,
//! exist, and parse to at least one skinned mesh? Diagnostic for "decapitated"
//! PC reports — a face value whose DAT is missing or mesh-less renders headless.

use ffxi_dat::archive::DatRoot;
use ffxi_dat::resource_dir::ResourceDir;

const FACE_BASES: [(&str, u32); 8] = [
    ("HumeM", 7080),
    ("HumeF", 10256),
    ("ElvaanM", 13432),
    ("ElvaanF", 16608),
    ("TarutaruM", 19784),
    ("TarutaruF", 22960),
    ("Mithra", 23184),
    ("Galka", 26360),
];

fn main() {
    let root = DatRoot::from_env_or_default().expect("FFXI_DAT_PATH or default install");
    for (race_idx, (name, base)) in FACE_BASES.iter().enumerate() {
        let mut ok = 0usize;
        let mut problems: Vec<String> = Vec::new();
        for face in 0u32..32 {
            let file_id = base + face;
            let loc = match root.resolve(file_id) {
                Ok(l) => l,
                Err(e) => {
                    problems.push(format!("face {face} fid {file_id}: resolve err {e}"));
                    continue;
                }
            };
            let path = loc.path_under(&root);
            let bytes = match std::fs::read(&path) {
                Ok(b) => b,
                Err(e) => {
                    problems.push(format!(
                        "face {face} fid {file_id}: unreadable {} ({e})",
                        path.display()
                    ));
                    continue;
                }
            };
            let meshes = ResourceDir::from_bytes(bytes).collect_skel_meshes();
            if meshes.is_empty() {
                problems.push(format!(
                    "face {face} fid {file_id}: 0 meshes ({})",
                    path.display()
                ));
            } else {
                ok += 1;
            }
        }
        println!("race {} {name}: {ok}/32 ok", race_idx + 1);
        for p in &problems {
            println!("  {p}");
        }
    }
}
