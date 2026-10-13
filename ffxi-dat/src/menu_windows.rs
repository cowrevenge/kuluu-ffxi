//! Menu window records inside the UI DATs — the §16 mini-chunk law and the
//! §43 `windowpsNNN` caption law from `vault main_menu.md`, turned into a reader.
//!
//! A DAT of this family is a chain of chunks; each chunk carries one window:
//! header `[fourcc 4][u32 kind+size]` (`kind = v & 0x7F`,
//! `size = ((v >> 7) & 0xFFFFF) * 16`; the law shared with the vault N-pass
//! scanner), and the window's own record name `menu    <root>` sits at chunk
//! offset +0x10. The records following it are variable-length, so children are
//! located by their names inside the chunk body: `menu    windowps` followed
//! by a **digit-run caption id ended by NUL** — up to five ASCII digits
//! (`windowps165`, `windowps1092`; whose value is the caption id into catalog
//! table 11 = ROM/165/75.DAT, the §43 law read from the conf-class GetText
//! sweep, FFXiMain.dll retail-2026-09 RVA 0x196ED1 `push 0x8f; push 0xb`). A
//! suffix that is not ASCII digits (a small binary id) marks a chrome/anchor
//! entry, which §43 excludes from row tables. Repeated caption ids inside one
//! chunk are separate records — retail pages genuinely show the same option
//! twice (e.g. conf7 shows 219 and 222 twice each; measured in vault §57).
//!
//! No DAT generation is ever hand-picked: [`windows_from_root`] consults the
//! candidate UI DAT files newest-first and matches windows by **record root**,
//! so looking up `conf6win` can only resolve in the named-root generation
//! (vault main_menu.md §49) while short-root-only windows such as `conf10`
//! fall back to the legacy base DAT. Each candidate id resolves through the
//! install's own VTABLE/FTABLE via [`DatRoot::resolve`] — the client's
//! mechanism, per the owner's no-stale-reads rule.

use std::collections::BTreeMap;

use crate::archive::DatRoot;
use crate::ui_element::UI_SHEET_FILE_ID;
use crate::{DatError, Result};

/// The legacy base UI DAT (`ROM/0/1.DAT` on every measured install) — the
/// short-root generation of vault main_menu.md §47. Last candidate: windows
/// whose roots exist only there (e.g. `conf10`) still resolve.
pub const LEGACY_UI_DAT_FILE_ID: u32 = 1;

/// The named-root window DAT — same file as [`UI_SHEET_FILE_ID`]
/// (`ROM/119/51.DAT` on horizonxi-2023 and retail-2026-09, whose
/// config-family chunks vault main_menu.md §49 pins). First candidate: the
/// newer generation wins for any root it carries.
pub const NAMED_ROOT_UI_DAT_FILE_ID: u32 = UI_SHEET_FILE_ID;

/// Catalog table 11 (`ROM/165/75.DAT`, EN, `parse_d_msg` n=1099 — the GetText
/// table behind every `windowpsNNN` caption, vault main_menu.md §43).
pub const CAPTION_CATALOG_TABLE_11_FILE_ID: u32 = 55651;

/// Candidate list [`windows_from_root`] consults, newest first.
pub const MENU_WINDOW_DAT_FILE_IDS: &[u32] = &[NAMED_ROOT_UI_DAT_FILE_ID, LEGACY_UI_DAT_FILE_ID];

const WINDOW_PREFIX: &[u8] = b"menu    ";
const WIDGET_NAME: &[u8] = b"menu    windowps";

/// §43 suffix law bound: caption ids are at most five ASCII digits before the
/// NUL (highest measured on retail-2026-09: 1096, vault main_menu.md §57).
const MAX_CAPTION_DIGITS: usize = 5;

/// One child widget located inside a window's chunk. `caption_id` is set for
/// the ASCII-digit `windowpsNNN` widgets (catalog table-11 caption id, §43);
/// binary-id chrome entries leave it `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowWidget {
    pub offset: usize,
    pub caption_id: Option<u32>,
}

/// One window record (one mini-chunk) and its children in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuWindow {
    /// Chunk-header fourcc naming this window's block (`con7` for `conf3win`;
    /// §16 warns fourccs are not always name prefixes).
    pub fourcc: String,
    /// The record root — the exact name FFXiMain.dll's menu manager opens.
    pub root: String,
    pub offset: usize,
    pub size: usize,
    pub widgets: Vec<WindowWidget>,
}

impl MenuWindow {
    /// The `windowpsNNN` caption ids in file order — the page's row set and
    /// its option-store keys (vault main_menu.md §43/§47). Repeated ids are
    /// repeated rows, kept.
    pub fn option_row_ids(&self) -> Vec<u32> {
        self.widgets.iter().filter_map(|w| w.caption_id).collect()
    }

    /// Rows as catalog strings; a caption missing from the table falls back to
    /// its id so a row is never silently dropped.
    pub fn option_rows(&self, mut catalog: impl FnMut(usize) -> Option<String>) -> Vec<String> {
        self.option_row_ids()
            .into_iter()
            .map(|id| {
                let text = usize::try_from(id).ok().and_then(&mut catalog);
                text.unwrap_or_else(|| format!("windowps{id}"))
            })
            .collect()
    }
}

/// Chunk headers along the chain law (vault N-pass tool 3): stop at the first
/// header with a short chunk or one that runs past EOF.
fn chunk_headers(bytes: &[u8]) -> Vec<(String, usize, usize)> {
    let mut out = Vec::new();
    let mut off = 0usize;
    while off + 16 <= bytes.len() {
        let Ok(name) = std::str::from_utf8(&bytes[off..off + 4]) else {
            break;
        };
        let v = u32::from_le_bytes([
            bytes[off + 4],
            bytes[off + 5],
            bytes[off + 6],
            bytes[off + 7],
        ]);
        // Masked to the DAT's 20-bit size field, so it always fits a usize.
        let size = ((v >> 7) & 0xFFFFF) as usize * 16;
        if size < 16 || off + size > bytes.len() {
            break;
        }
        out.push((name.to_string(), off, size));
        off += size;
    }
    out
}

/// `windowps` suffix law: ASCII digits ended by NUL → caption id; otherwise a
/// small binary id (chrome/anchor, §43) → `None`. The slice runs past the
/// longest possible digit run so 4- and 5-digit ids keep their terminator.
fn parse_widget_suffix(suffix: &[u8]) -> Option<u32> {
    let end = suffix.iter().position(|b| *b == 0)?;
    if end == 0 || end > MAX_CAPTION_DIGITS || !suffix[..end].iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(&suffix[..end]).ok()?.parse().ok()
}

/// Parse every window record in one UI DAT file's bytes.
pub fn parse_windows(bytes: &[u8]) -> Vec<MenuWindow> {
    let mut out = Vec::new();
    for (fourcc, off, size) in chunk_headers(bytes) {
        // The window's own record name sits at +0x10 (§16 mini-chunk shape).
        let Some(head) = bytes.get(off + 0x10..off + 0x20) else {
            continue;
        };
        if &head[..WINDOW_PREFIX.len()] != WINDOW_PREFIX {
            continue;
        }
        // The manager's 16-byte name field keeps `menu    ` on the front and
        // space-pads (or NUL-terminates) the short root behind it.
        let rest = &head[WINDOW_PREFIX.len()..];
        let padded = rest.iter().position(|b| *b == 0).unwrap_or(rest.len());
        let Ok(root_full) = std::str::from_utf8(&rest[..padded]) else {
            continue;
        };
        let root = root_full.trim_end().to_string();
        if !root.as_bytes().first().is_some_and(u8::is_ascii_alphabetic) {
            continue;
        }
        // Children: widget names inside the body, located by scan (records are
        // variable-length and not quad-aligned in a stable way).
        let body_start = off + 0x20;
        let body_end = off + size;
        let mut widgets = Vec::new();
        let mut from = body_start;
        while from < body_end {
            let Some(rel) = find_in_range(bytes, body_end, WIDGET_NAME, from) else {
                break;
            };
            let after_name = rel + WIDGET_NAME.len();
            // The NUL of a digit id can sit up to MAX_CAPTION_DIGITS bytes in.
            let look_end = (after_name + 1 + MAX_CAPTION_DIGITS).min(bytes.len());
            widgets.push(WindowWidget {
                offset: rel,
                caption_id: parse_widget_suffix(&bytes[after_name..look_end]),
            });
            from = after_name;
        }
        out.push(MenuWindow {
            fourcc,
            root,
            offset: off,
            size,
            widgets,
        });
    }
    out
}

fn find_in_range(hay: &[u8], end: usize, needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || end > hay.len() {
        return None;
    }
    let last_start = end - needle.len();
    (from..=last_start).find(|i| &hay[*i..*i + needle.len()] == needle)
}

/// All windows of the candidate menu DATs keyed by record root; newest file
/// wins per root ([`MENU_WINDOW_DAT_FILE_IDS`] is newest-first).
pub fn windows_from_root(dat: &DatRoot) -> Result<BTreeMap<String, MenuWindow>> {
    let mut map = BTreeMap::new();
    // Walk oldest→newest and overwrite, so the newest file wins for a root.
    for &file_id in MENU_WINDOW_DAT_FILE_IDS.iter().rev() {
        let loc = dat.resolve(file_id)?;
        let path = loc.path_under(dat);
        let bytes = std::fs::read(&path).map_err(|source| DatError::Io { path, source })?;
        for window in parse_windows(&bytes) {
            map.insert(window.root.clone(), window);
        }
    }
    if map.is_empty() {
        return Err(DatError::Io {
            path: dat.root().to_path_buf(),
            source: std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no menu window record parsed from any candidate UI DAT",
            ),
        });
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A full chunk: header, the §16 zero-pad up to +0x10 (where the window
    /// record name sits), then the body, padded up to the declared size.
    fn chunk(fourcc: &[u8; 4], mut body: Vec<u8>) -> Vec<u8> {
        let size_words = (body.len() + 0x10).div_ceil(16) as u32;
        let mut out = fourcc.to_vec();
        out.extend_from_slice(&(size_words << 7 | 0x30u32).to_le_bytes());
        out.resize(0x10, 0);
        out.append(&mut body);
        let target = size_words as usize * 16;
        out.resize(target.max(out.len()), 0);
        out
    }

    /// Root record: the padded window name at +0x10 plus the §16 geometry word-run.
    fn root_record(root: &str) -> Vec<u8> {
        let mut r = format!("menu    {root}").into_bytes();
        while r.len() % 4 != 0 || r.len() < 0x10 {
            // keep the name field's padded tail quad-clean
            r.push(0);
        }
        r.extend_from_slice(&[0x50, 0x00, 0x10, 0x00, 0x30, 0x00, 0x00, 0x00]);
        r
    }

    /// A widget record as the real chunks pack them: previous-record tail, then
    /// `menu    windowpsNNN`, anchor `-1`, geometry words.
    fn widget(id: &str) -> Vec<u8> {
        let mut r = vec![0x00, 0x03, 0x02, 0x00];
        r.extend_from_slice(format!("menu    windowps{id}\0").as_bytes());
        r.extend_from_slice(b"-1\0");
        r.extend_from_slice(&[0x40, 0x00, 0xAE, 0x00, 0x3F, 0x00, 0x00, 0x00]);
        r
    }

    #[test]
    fn parses_a_synthetic_named_root_window() {
        // Shaped after ROM/119/51.DAT chunk 'con7' @0x28AC00 (vault §49).
        let mut body = root_record("conf3win");
        for id in ["165", "166", "181"] {
            body.extend_from_slice(&widget(id));
        }
        // A binary-id chrome widget (§43): no ASCII, must parse without caption.
        let mut chrome = vec![0x00, 0x03, 0x02, 0x00];
        chrome.extend_from_slice(b"menu    windowps\x06\0");
        body.extend_from_slice(&chrome);

        let bytes = chunk(b"con7", body);
        let windows = parse_windows(&bytes);
        assert_eq!(windows.len(), 1, "one window per mini-chunk");
        let w = &windows[0];
        assert_eq!((w.fourcc.as_str(), w.root.as_str()), ("con7", "conf3win"));
        // Caption ids in file order; the chrome entry contributes none.
        assert_eq!(w.option_row_ids(), vec![165, 166, 181]);

        let rows = w.option_rows(|i| Some(format!("cap{i}")));
        assert_eq!(
            rows,
            vec!["cap165".to_string(), "cap166".into(), "cap181".into()]
        );
    }

    #[test]
    fn multi_digit_ids_keep_their_terminator() {
        // co22 'conf13wi' carries windowps1088 (vault §57): the NUL lands past
        // a naive 4-byte suffix slice.
        let mut body = root_record("conf13wi");
        for id in ["986", "147", "148", "1088", "1090", "1089"] {
            body.extend_from_slice(&widget(id));
        }
        let bytes = chunk(b"co22", body);
        assert_eq!(
            parse_windows(&bytes)[0].option_row_ids(),
            vec![986, 147, 148, 1088, 1090, 1089]
        );
    }

    #[test]
    fn suffix_law_separates_ids_from_binary_chrome() {
        assert_eq!(parse_widget_suffix(b"165\0"), Some(165));
        assert_eq!(parse_widget_suffix(b"985\0"), Some(985));
        assert_eq!(parse_widget_suffix(b"1088\0"), Some(1088));
        assert_eq!(parse_widget_suffix(b"\x06\0\0\0"), None); // chrome, §43
        assert_eq!(parse_widget_suffix(b"\0\0\0\0"), None);
        assert_eq!(parse_widget_suffix(b"12\x00\x00"), Some(12));
        // Six digits is beyond the measured id space — treat as not a caption.
        assert_eq!(parse_widget_suffix(b"123456\0"), None);
    }

    #[test]
    fn chunk_chain_stops_at_a_bogus_header() {
        let mut bytes = chunk(b"con7", root_record("conf3win"));
        // A second chunk header claiming a size past EOF must end the walk.
        let n = bytes.len();
        bytes.resize(n + 16, 0);
        bytes[n..n + 4].copy_from_slice(b"bad!");
        bytes[n + 4..n + 8].copy_from_slice(&(999_999u32 << 7 | 0x30u32).to_le_bytes());
        let windows = parse_windows(&bytes);
        assert_eq!(windows.len(), 1);
    }
}
