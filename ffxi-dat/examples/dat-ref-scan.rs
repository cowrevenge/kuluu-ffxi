use std::process::ExitCode;

use ffxi_dat::chunk::walk;
use ffxi_dat::kind::ChunkKind;
use ffxi_dat::DatRoot;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: dat-ref-scan <file_id> <needle> [needle...]");
        return ExitCode::from(1);
    }
    let file_id: u32 = args[0].parse().expect("file_id");
    let needles: Vec<&[u8]> = args[1..].iter().map(|s| s.as_bytes()).collect();
    let root = DatRoot::from_env_or_default().expect("dat root");
    let loc = root.resolve(file_id).expect("resolve");
    let path = loc.path_under(root.root());
    let bytes = std::fs::read(&path).expect("read");
    for (idx, c) in walk(&bytes).filter_map(Result::ok).enumerate() {
        for needle in &needles {
            let mut at = Vec::new();
            let mut i = 0;
            while i + needle.len() <= c.data.len() {
                if &c.data[i..i + needle.len()] == *needle {
                    at.push(i);
                }
                i += 1;
            }
            if !at.is_empty() {
                println!(
                    "chunk[{idx}] kind=0x{:02X} ({:?}) name={:?} len={} contains {:?} at {:?}",
                    c.kind,
                    ChunkKind::label(c.kind),
                    String::from_utf8_lossy(&c.name).trim_end(),
                    c.data.len(),
                    String::from_utf8_lossy(needle),
                    &at[..at.len().min(8)]
                );
            }
        }
    }
    ExitCode::SUCCESS
}
