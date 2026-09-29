//! Lamp-glow diagnostics for a zone's effect DAT: dumps every generator def under `ligh/`
//! (mesh, init colour/scale, blend) plus the alpha/hue statistics of the lig* halo textures
//! and sheet frame extents — the data needed to judge how a lamp's wall glow should render.

use std::env;
use std::fs;
use std::process::ExitCode;

use ffxi_dat::{
    chunk, kind::ChunkKind, particle_gen::ParticleGeneratorDef, sprite_sheet::ParticleSpriteSheet,
    texture, DatRoot,
};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let Some(arg) = args.get(1) else {
        eprintln!("usage: dat-lamp-glow-probe <file_id>");
        return ExitCode::FAILURE;
    };
    let Ok(file_id) = arg.parse::<u32>() else {
        eprintln!("bad file id {arg:?}");
        return ExitCode::FAILURE;
    };
    let root = DatRoot::from_env().unwrap();
    let location = root.resolve(file_id).unwrap();
    println!("{} -> {}", file_id, location.path_under(&root).display());
    let bytes = fs::read(location.path_under(&root)).unwrap();

    let mut out = String::new();
    visit(&chunk::walk_tree(&bytes), [0; 4], &mut out);
    print!("{out}");
    ExitCode::SUCCESS
}

fn visit(node: &chunk::ChunkNode, dir: [u8; 4], out: &mut String) {
    for child in &node.children {
        if child.chunk.kind == 0x01 {
            visit(child, child.chunk.name, out);
            continue;
        }
        let name = child.chunk.name_str();
        match ChunkKind::from_u8(child.chunk.kind) {
            Some(ChunkKind::Generator) if dir == *b"ligh" => {
                if let Ok(Some(d)) = ParticleGeneratorDef::parse(child.chunk.data) {
                    out.push_str(&format!(
                        "gen {:?}/{:?} kind={:?} mesh={:?} init_color=({:.3},{:.3},{:.3},{:.3}) \
                         init_scale=({:.2},{:.2},{:.2}) blend={:?} billboard={:?} life={}f\n",
                        dir,
                        child.chunk.name,
                        d.mesh_kind,
                        d.mesh_id,
                        d.init_color[0],
                        d.init_color[1],
                        d.init_color[2],
                        d.init_color[3],
                        d.init_scale[0],
                        d.init_scale[1],
                        d.init_scale[2],
                        d.blend,
                        d.billboard,
                        d.max_life_frames.round() as u32,
                    ));
                } else {
                    out.push_str(&format!(
                        "gen {:?}/{:?} (no particle def)\n",
                        dir, child.chunk.name
                    ));
                }
            }
            Some(ChunkKind::Img) if name.starts_with("lig") => {
                match texture::decode_texture(child.chunk.data) {
                    Ok(t) => image_stats(&name, &t, out),
                    Err(e) => out.push_str(&format!("img {name}: decode err {e}\n")),
                }
            }
            Some(ChunkKind::SpriteSheet) if name.starts_with("lig") => {
                if let Some(ss) = ParticleSpriteSheet::parse(child.chunk.data) {
                    for f in &ss.frames {
                        if f.positions.is_empty() {
                            continue;
                        }
                        let xs: Vec<f32> = f.positions.iter().map(|p| p[0]).collect();
                        let ys: Vec<f32> = f.positions.iter().map(|p| p[1]).collect();
                        out.push_str(&format!(
                            "sheet {:?} ({}:{}) frames={} quads={} pos=[{:.2},{:.2}]..[{:.2},{:.2}]\n",
                            name, ss.category, ss.id, ss.frames.len(), f.positions.len() / 4,
                            xs.iter().cloned().fold(f32::INFINITY, f32::min),
                            ys.iter().cloned().fold(f32::INFINITY, f32::min),
                            xs.iter().cloned().fold(f32::NEG_INFINITY, f32::max),
                            ys.iter().cloned().fold(f32::NEG_INFINITY, f32::max),
                        ));
                    }
                }
            }
            Some(ChunkKind::D3m) if name.starts_with("li") || name.starts_with("ghu") => {
                if let Ok(d) = ffxi_dat::d3m::D3m::parse(child.chunk.name, child.chunk.data) {
                    out.push_str(&format!("(dir={:?}) ", dir));
                    d3m_color_stats(&name, &d, out);
                }
            }
            _ => {}
        }
    }
}

// The fixture mesh is untextured: its whole look is vertex colour x TFACTOR (the D3m
// untextured table), so the authored vertex range IS the glass glow.
fn d3m_color_stats(name: &str, d: &ffxi_dat::d3m::D3m, out: &mut String) {
    let n = d.vertices.len() as f64;
    if n == 0.0 {
        return;
    }
    let mut sum = [0f64; 4];
    let (mut lo, mut hi) = ([f64::INFINITY; 4], [f64::NEG_INFINITY; 4]);
    for v in &d.vertices {
        for c in 0..4 {
            let x = v.color[c] as f64;
            sum[c] += x;
            lo[c] = lo[c].min(x);
            hi[c] = hi[c].max(x);
        }
    }
    out.push_str(&format!(
        "d3m {name} verts={} rgb_a=[lo {:.2}/{:.2}/{:.2}/{:.2}  mean {:.2}/{:.2}/{:.2}/{:.2}  hi {:.2}/{:.2}/{:.2}/{:.2}]\n",
        d.vertices.len(),
        lo[0], lo[1], lo[2], lo[3],
        sum[0] / n, sum[1] / n, sum[2] / n, sum[3] / n,
        hi[0], hi[1], hi[2], hi[3],
    ));
}

fn image_stats(name: &str, t: &texture::DecodedTexture, out: &mut String) {
    let mut alphas = Vec::with_capacity(t.rgba.len() / 4);
    let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
    for px in t.rgba.chunks_exact(4) {
        alphas.push(px[3]);
        if px[3] > 128 {
            r += u64::from(px[0]);
            g += u64::from(px[1]);
            b += u64::from(px[2]);
            n += 1;
        }
    }
    alphas.sort_unstable();
    let q = |p: usize| alphas.get(alphas.len() * p / 100).copied().unwrap_or(0);
    out.push_str(&format!(
        "img {name} {}x{} alpha[min/p25/med/p75/max]={}/{}/{}/{}/{} core_rgb=({:.3},{:.3},{:.3}) n={}\n",
        t.width, t.height, q(0), q(25), q(50), q(75), q(99),
        if n > 0 { r as f32 / n as f32 } else { 0.0 },
        if n > 0 { g as f32 / n as f32 } else { 0.0 },
        if n > 0 { b as f32 / n as f32 } else { 0.0 },
        n,
    ));
}
