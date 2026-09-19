use std::process::ExitCode;

use ffxi_dat::zone_dat::zone_id_to_mzb_file_id;
use ffxi_dat::zone_interaction::{self, ZoneInteraction};
use ffxi_dat::DatRoot;

fn tag(r: &ZoneInteraction) -> String {
    String::from_utf8_lossy(&r.source_id.0).to_string()
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(zone_id) = args.first().and_then(|s| s.parse::<u16>().ok()) else {
        eprintln!("usage: dat-rid-zoneline-probe <zone_id>");
        return ExitCode::from(1);
    };
    let root = DatRoot::from_env_or_default().expect("dat root");
    let file_id = zone_id_to_mzb_file_id(zone_id).expect("zone has no MZB file id");
    let loc = root.resolve(file_id).expect("resolve");
    let bytes = std::fs::read(loc.path_under(&root)).expect("read");
    let all = zone_interaction::from_dat(&bytes).expect("RID parse");

    println!("zone {zone_id} (file {file_id}): {} RID entries", all.len());
    println!(
        "{:<6} {:>6} {:>9} {:>9} {:>9} {:>9} {:>8} {:>8} {:>8} {:>9} {:>9}",
        "tag",
        "class",
        "pos.x",
        "pos.y",
        "pos.z",
        "yaw",
        "size.x",
        "size.y",
        "size.z",
        "rot.x",
        "rot.z"
    );
    for r in all.iter().filter(|r| r.is_zone_line()) {
        println!(
            "{:<6} {:>6} {:>9.3} {:>9.3} {:>9.3} {:>9.6} {:>8.3} {:>8.3} {:>8.3} {:>9.6} {:>9.6}",
            tag(r),
            r.rect_class,
            r.position[0],
            r.position[1],
            r.position[2],
            r.orientation[1],
            r.size[0],
            r.size[1],
            r.size[2],
            r.orientation[0],
            r.orientation[2],
        );
    }
    let entrances = all.iter().filter(|r| r.is_zone_entrance()).count();
    println!("zone-entrance ('z' with zero dest) rects: {entrances}");
    ExitCode::SUCCESS
}
