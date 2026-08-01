use std::process::ExitCode;

use ffxi_dat::chunk::{walk_tree, Chunk, ChunkNode};
use ffxi_dat::kind::ChunkKind;
use ffxi_dat::particle_gen::{AttachType, ParticleGeneratorDef, ParticleMeshKind};
use ffxi_dat::DatRoot;

fn id(name: &[u8; 4]) -> String {
    String::from_utf8_lossy(name).trim_end().to_string()
}

// Bounding extents of a celestial billboard's source mesh, which is what sizes the drawn
// disc in retail (there is no disc-radius constant). D3M for StaticMesh, 0x21 frame 0 for
// SpriteSheet.
fn mesh_extents(bytes: &[u8], mesh_id: &[u8; 4], kind: ParticleMeshKind) -> Option<String> {
    for c in ffxi_dat::chunk::walk(bytes).flatten() {
        if c.name != *mesh_id {
            continue;
        }
        match (ChunkKind::from_u8(c.kind), kind) {
            (Some(ChunkKind::D3m), ParticleMeshKind::StaticMesh) => {
                let d3m = ffxi_dat::d3m::D3m::parse(c.name, c.data).ok()?;
                let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
                for v in &d3m.vertices {
                    for a in 0..3 {
                        lo[a] = lo[a].min(v.pos[a]);
                        hi[a] = hi[a].max(v.pos[a]);
                    }
                }
                let (ns, local) = d3m.texture_name_tokens();
                return Some(format!(
                    "d3m verts={} extent=[{:.2} x {:.2} x {:.2}] tex={ns}/{local}",
                    d3m.vertices.len(),
                    hi[0] - lo[0],
                    hi[1] - lo[1],
                    hi[2] - lo[2],
                ));
            }
            (Some(ChunkKind::SpriteSheet), ParticleMeshKind::SpriteSheet) => {
                let ss = ffxi_dat::sprite_sheet::ParticleSpriteSheet::parse(c.data)?;
                let f = ss.frames.first()?;
                let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
                for p in &f.positions {
                    for a in 0..3 {
                        lo[a] = lo[a].min(p[a]);
                        hi[a] = hi[a].max(p[a]);
                    }
                }
                return Some(format!(
                    "sheet frames={} extent=[{:.2} x {:.2} x {:.2}] tex={}/{}",
                    ss.frames.len(),
                    hi[0] - lo[0],
                    hi[1] - lo[1],
                    hi[2] - lo[2],
                    ss.category,
                    ss.id,
                ));
            }
            // The sun/moon billboards link an MMB, not a D3M, despite the StaticMesh kind:
            // resolve_mesh's D3M lookup misses and the MMB fallback is what draws them.
            (Some(ChunkKind::Mmb), ParticleMeshKind::StaticMesh) => {
                // The sun billboards link an MMB despite the StaticMesh kind: resolve_mesh's
                // D3M lookup misses and particle_sim's MMB fallback is what draws them.
                let dec = ffxi_dat::mmb::decrypt(c.data).ok()?;
                let models = ffxi_dat::mmb::parse_models(&dec);
                let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
                let mut verts = 0usize;
                for p in models.iter().flat_map(|m| m.vertices.iter()) {
                    verts += 1;
                    for a in 0..3 {
                        lo[a] = lo[a].min(p.pos[a]);
                        hi[a] = hi[a].max(p.pos[a]);
                    }
                }
                if verts == 0 {
                    return Some("mmb (no vertices)".to_string());
                }
                return Some(format!(
                    "mmb verts={verts} extent=[{:.3} x {:.3} x {:.3}]",
                    hi[0] - lo[0],
                    hi[1] - lo[1],
                    hi[2] - lo[2],
                ));
            }
            _ => {}
        }
    }
    None
}

fn keyframe_track(bytes: &[u8], track: &[u8; 4]) -> Option<ffxi_dat::particle_gen::KeyFrameTrack> {
    ffxi_dat::chunk::walk(bytes)
        .flatten()
        .find(|c| c.name == *track && ChunkKind::from_u8(c.kind) == Some(ChunkKind::KeyFrame))
        .map(|c| ffxi_dat::particle_gen::KeyFrameTrack::parse(c.data))
}

fn visit(node: &ChunkNode<'_>, path: &mut Vec<String>, bytes: &[u8], hits: &mut usize) {
    for child in &node.children {
        let c: &Chunk = &child.chunk;
        if !child.children.is_empty() || c.kind == 0x01 {
            path.push(id(&c.name));
            visit(child, path, bytes, hits);
            path.pop();
            continue;
        }
        if ChunkKind::from_u8(c.kind) != Some(ChunkKind::Generator) {
            continue;
        }
        let Ok(Some(def)) = ParticleGeneratorDef::parse(c.data) else {
            continue;
        };
        if !matches!(def.attach_type, AttachType::Sun | AttachType::Moon) {
            continue;
        }
        *hits += 1;
        println!(
            "  {}/{:<5} {:?}  mesh={:<4} {:?} blend={:?} life={} auto={} bb={} cont={}",
            path.join("/"),
            id(&c.name),
            def.attach_type,
            id(&def.mesh_id),
            def.mesh_kind,
            def.blend,
            def.max_life_frames,
            def.auto_run,
            def.camera_billboard,
            def.continuous,
        );
        println!(
            "        color={:?} scale={:?} tracks(sx,sy,a)={:?} dow={} phase={}",
            def.init_color,
            def.init_scale,
            (
                def.scale_x_track.map(|t| id(&t)),
                def.scale_y_track.map(|t| id(&t)),
                def.alpha_track.map(|t| id(&t)),
            ),
            def.day_of_week_color.is_some(),
            def.moon_phase_color.is_some(),
        );
        println!(
            "        tod(r,g,b,a)={:?} driven={:?} moon_phase_sprite={}",
            def.tod_color_tracks.map(|t| t.map(|t| id(&t))),
            def.tod_color_driven,
            def.moon_phase_sprite,
        );
        if let Some(t) = def.day_of_week_color {
            let cols: Vec<String> = t
                .iter()
                .map(|c| format!("({:.2},{:.2},{:.2})", c[0], c[1], c[2]))
                .collect();
            println!("        0x4E dow  {}", cols.join(" "));
        }
        if let Some(t) = def.moon_phase_color {
            let cols: Vec<String> = t
                .iter()
                .map(|c| format!("({:.2},{:.2},{:.2})", c[0], c[1], c[2]))
                .collect();
            println!("        0x4F phase {}", cols.join(" "));
        }
        for (chan, track) in ["r", "g", "b", "a"].iter().zip(def.tod_color_tracks) {
            let Some(track) = track else { continue };
            let Some(curve) = keyframe_track(bytes, &track) else {
                continue;
            };
            let hours: Vec<String> = curve
                .points
                .iter()
                .map(|(t, v)| format!("{:.1}h={v:.2}", t * 24.0))
                .collect();
            println!("        tod.{chan} {} -> {}", id(&track), hours.join(" "));
        }
        match mesh_extents(bytes, &def.mesh_id, def.mesh_kind) {
            Some(m) => println!("        {m}"),
            None => println!("        (mesh {} unresolved)", id(&def.mesh_id)),
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: FFXI_DAT_PATH=<path> dat-celestial-probe <file_id>...");
        return ExitCode::from(2);
    }
    let root = match DatRoot::from_env_or_default() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("could not open DAT root: {e}");
            return ExitCode::from(1);
        }
    };
    for arg in &args {
        let Ok(file_id) = arg.parse::<u32>() else {
            continue;
        };
        let Ok(loc) = root.resolve(file_id) else {
            eprintln!("{file_id}: unresolvable");
            continue;
        };
        let Ok(bytes) = std::fs::read(loc.path_under(root.root())) else {
            eprintln!("{file_id}: unreadable");
            continue;
        };
        println!("== file {file_id} ({} bytes)", bytes.len());
        let tree = walk_tree(&bytes);
        let mut hits = 0usize;
        visit(&tree, &mut Vec::new(), &bytes, &mut hits);
        println!("   {hits} Sun/Moon-attached generator(s)");
    }
    ExitCode::SUCCESS
}
