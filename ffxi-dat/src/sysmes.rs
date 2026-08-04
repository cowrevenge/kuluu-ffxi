//! The client's **system-message DialogTable** — the strings retail composes
//! locally rather than receiving as text, including every treasure-pool line.
//!
//! Same container format as [`crate::dmsg::StringDat`]; what differs is the
//! control-code grammar, which carries substitution slots the zone dialog
//! tables do not use. Located at `ROM/27/76.DAT` in the NA install (empirical —
//! found by scanning for the pool wording, like the emote table next to it at
//! `ROM/27/70.DAT`), so [`SysMesDat::open`] validates the shape rather than
//! trusting the path: entry 262 of a real table is the untranslated placeholder
//! `sysmes262`.
//!
//! Composition returns spans, not a flat string, because retail colours the
//! item-name substitution differently from the text around it — "You find a
//! [pair of bounding boots] on Leaping Lizzy." renders the bracketed item green
//! against white (retail screenshots, 2026-08-03; see
//! `.agents/skills/retail-observe/references/`).

use crate::dmsg::{
    self, parse_inline_tag, split_alternative, StringDat, ALT_OPEN, CC_AUTO, CC_INLINE_TAG,
    CC_NEWLINE, CC_NUM, MARKER_ITEM, MARKER_KEY_ITEM, PRINTABLE,
};

/// `<install root>/ROM/27/76.DAT` (FTABLE sub_path dir 27, file 76).
pub const SYS_MES_SUB_PATH: (u16, u8) = (27, 76);

/// Entry whose NA text is the untranslated placeholder `sysmes262`, used to
/// tell a real system-message table from any other DialogTable that happens to
/// parse.
const SHAPE_PROBE_INDEX: usize = 262;
const SHAPE_PROBE_TEXT: &str = "sysmes";

/// Leading `0x1F <mode>`: the retail chat-log message type, which selects the
/// line's colour from the player's Config → Font Colors. Distinct from `0x1E`
/// (`CC_SET_COLOR`), which sets an inline colour and appears nowhere in this
/// table.
const CC_LOG_MODE: u8 = 0x1f;
/// `0x1C <n>`: text parameter `n`. POLUtils names this code `ChocoboName`; in
/// the system-message table it is plainly a generic string slot (player names,
/// and the lot value in entry 17).
const CC_TEXT_PARAM: u8 = 0x1c;
/// `0x12 <n>`: numeric parameter `n`, a second family alongside [`CC_NUM`].
const CC_NUM2: u8 = 0x12;

/// `0x01 0x01 <slot>` — a bare substitution slot, sharing its prefix with the
/// emote table's caster/target slots (0x10/0x11).
const SLOT_PREFIX: [u8; 2] = [0x01, 0x01];
/// The a/an article chosen for the item named by the next inline item tag.
const SLOT_ARTICLE: u8 = 0x01;
/// The entity the message is about — the mob or object that dropped the item.
const SLOT_TARGET_NAME: u8 = 0x11;

// `0x7F <kind> [<param>]` sequences. The emote table's caster-emphasis pair
// (0xFC/0xFB), article alternative (0x88) and terminator (0x31) are shared;
// see [`crate::dmsg`] for those.
/// Capitalize the next substitution.
const AUTO_CAPITALIZE: u8 = 0x80;
/// `[singular/plural]` chosen by numeric parameter `n`.
const AUTO_PLURAL_WORD: u8 = 0x86;
/// `[/s]` — the bare plural suffix, chosen by numeric parameter `n`.
const AUTO_PLURAL_SUFFIX: u8 = 0x92;
/// A gil amount from numeric parameter `n`, rendered with its unit.
const AUTO_GIL: u8 = 0xb4;

/// Retail writes gil amounts with thousands separators.
const GIL_GROUP_DIGITS: usize = 3;
const GIL_UNIT: &str = "gil";

/// Substitution slots addressable by a `<n>` parameter byte. The table's
/// highest observed reference is 4 (entry 219's chevron counts).
pub const PARAM_SLOTS: usize = 8;

/// Which retail colour a composed span takes. Only the item-name family is
/// coloured apart from the line's log-mode colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanKind {
    Text,
    Item,
    KeyItem,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub kind: SpanKind,
}

/// One composed system message. `lines` holds one entry per retail log line —
/// a `0x07` inside an entry starts a new one rather than wrapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SysMesLine {
    pub log_mode: Option<u8>,
    pub lines: Vec<Vec<Span>>,
}

impl SysMesLine {
    /// The whole message as plain text, log lines joined by `\n`. For tests and
    /// for consumers that have nowhere to put colour.
    pub fn to_plain(&self) -> String {
        self.lines
            .iter()
            .map(|spans| spans.iter().map(|s| s.text.as_str()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Values substituted into an entry's slots, indexed by the parameter byte the
/// entry references. `items` is pre-resolved by the caller because item-id →
/// name lives outside this crate.
#[derive(Debug, Default, Clone)]
pub struct SysMesParams<'a> {
    pub strings: [Option<&'a str>; PARAM_SLOTS],
    pub numbers: [i64; PARAM_SLOTS],
    pub items: [Option<&'a str>; PARAM_SLOTS],
    pub key_items: [Option<&'a str>; PARAM_SLOTS],
    /// Fills [`SLOT_TARGET_NAME`].
    pub target_name: Option<&'a str>,
    /// Keep the leading article of a `[the /]` alternative — cleared for a
    /// named entity, which retail refers to without "the".
    pub target_article: bool,
}

pub struct SysMesDat {
    dat: StringDat,
}

impl SysMesDat {
    pub fn open(root: &crate::DatRoot) -> Option<Self> {
        let (dir, file) = SYS_MES_SUB_PATH;
        let path = root
            .root()
            .join("ROM")
            .join(dir.to_string())
            .join(format!("{file}.DAT"));
        let bytes = std::fs::read(path).ok()?;
        let dat = StringDat::parse(&bytes).ok()?;
        dat.text(SHAPE_PROBE_INDEX)?
            .contains(SHAPE_PROBE_TEXT)
            .then_some(Self { dat })
    }

    pub fn len(&self) -> usize {
        self.dat.len()
    }

    pub fn is_empty(&self) -> bool {
        self.dat.is_empty()
    }

    pub fn message(&self, index: usize, params: &SysMesParams) -> Option<SysMesLine> {
        Some(compose(self.dat.raw(index)?, params))
    }
}

/// Indices of the treasure-pool messages, read out of the NA install's table.
/// The client picks between the pairs by packet flags: `IsContainer` selects
/// [`FIND_IN`] over [`FIND_ON`], and a zero `LootUniqueNo`/`EntryUniqueNo`
/// selects the first-person wording (research/XiPackets/world/server/0x00D3).
pub mod treasure {
    /// `<name> does not meet the necessary requirements to obtain the <item>.`
    /// plus a second line, `<item> lost.`
    pub const OTHER_INELIGIBLE: usize = 15;
    /// `You find a <item> on [the ]<mob>.`
    pub const FIND_ON: usize = 16;
    /// `<name>'s lot for the <item>: <n> points.`
    pub const LOT: usize = 17;
    /// `<name> obtains a <item>.`
    pub const OBTAINS_ITEM: usize = 18;
    /// `<name> obtains <n> gil.`
    pub const OBTAINS_GIL: usize = 19;
    /// `You do not meet the requirements to obtain the <item>.` plus `<item> lost.`
    pub const YOU_INELIGIBLE: usize = 31;
    /// `You cast lots for the <item>.`
    pub const YOU_CAST_LOTS: usize = 130;
    /// `You obtain a <item>.`
    pub const YOU_OBTAIN: usize = 131;
    /// `A <item> was lost.`
    pub const WAS_LOST: usize = 164;
    /// `You find a <item> in the <container>.`
    pub const FIND_IN: usize = 218;
}

/// Which `[a/b]` alternative the pending `0x7F` code selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Alt {
    None,
    /// `[the /]` — keep the first branch for an unnamed entity.
    Article,
    /// `[singular/plural]` or `[/s]`, decided by a numeric parameter.
    Plural(usize),
}

struct Composer {
    lines: Vec<Vec<Span>>,
    pending: String,
    alt: Alt,
    capitalize: bool,
}

impl Composer {
    fn new() -> Self {
        Self {
            lines: vec![Vec::new()],
            pending: String::new(),
            alt: Alt::None,
            capitalize: false,
        }
    }

    fn flush_text(&mut self) {
        if !self.pending.is_empty() {
            let text = std::mem::take(&mut self.pending);
            self.current().push(Span {
                text,
                kind: SpanKind::Text,
            });
        }
    }

    fn current(&mut self) -> &mut Vec<Span> {
        self.lines.last_mut().expect("always one open line")
    }

    fn push_text(&mut self, s: &str) {
        let s = self.take_capitalization(s);
        self.pending.push_str(&s);
    }

    fn push_span(&mut self, text: &str, kind: SpanKind) {
        if text.is_empty() {
            return;
        }
        let text = self.take_capitalization(text);
        self.flush_text();
        self.current().push(Span { text, kind });
    }

    fn take_capitalization(&mut self, s: &str) -> String {
        if !self.capitalize {
            return s.to_string();
        }
        self.capitalize = false;
        let mut chars = s.chars();
        match chars.next() {
            Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
            None => String::new(),
        }
    }

    fn newline(&mut self) {
        self.flush_text();
        self.lines.push(Vec::new());
    }

    fn finish(mut self) -> Vec<Vec<Span>> {
        self.flush_text();
        self.lines.retain(|l| !l.is_empty());
        self.lines
    }
}

fn compose(entry: &[u8], params: &SysMesParams) -> SysMesLine {
    let mut log_mode = None;
    let mut c = Composer::new();
    let mut i = 0;
    while i < entry.len() {
        let b = entry[i];

        if b == CC_LOG_MODE {
            log_mode = entry.get(i + 1).copied();
            i += 2;
            continue;
        }

        if b == CC_AUTO {
            let Some(&kind) = entry.get(i + 1) else {
                break;
            };
            let param = entry.get(i + 2).copied().unwrap_or(0) as usize;
            match kind {
                dmsg::AUTO_EMOTE_END => break,
                AUTO_CAPITALIZE => c.capitalize = true,
                dmsg::AUTO_EMOTE_TARGET_ARTICLE => c.alt = Alt::Article,
                AUTO_PLURAL_WORD | AUTO_PLURAL_SUFFIX => c.alt = Alt::Plural(param),
                AUTO_GIL => {
                    let n = params.numbers.get(param).copied().unwrap_or(0);
                    c.push_text(&format_gil(n));
                }
                // The caster-emphasis pair carries no parameter of its own.
                dmsg::AUTO_EMOTE_CASTER_OPEN | dmsg::AUTO_EMOTE_CASTER_CLOSE => {
                    i += 2;
                    continue;
                }
                _ => {}
            }
            i += 3;
            continue;
        }

        if entry[i..].starts_with(&SLOT_PREFIX) {
            match entry.get(i + 2) {
                Some(&SLOT_ARTICLE) => {
                    let article = article_for(next_item_name(entry, i, params).unwrap_or_default());
                    c.push_text(article);
                    i += 3;
                    continue;
                }
                Some(&SLOT_TARGET_NAME) => {
                    let name = params.target_name.unwrap_or_default().to_string();
                    c.push_text(&name);
                    i += 3;
                    continue;
                }
                _ => {}
            }
        }

        if b == CC_INLINE_TAG {
            // A malformed tag drops only the 0x01, like other control bytes.
            if let Some(tag) = parse_inline_tag(entry, i) {
                let slot = tag.param as usize;
                match tag.marker {
                    Some(MARKER_ITEM) => {
                        let name = params
                            .items
                            .get(slot)
                            .copied()
                            .flatten()
                            .unwrap_or_default();
                        c.push_span(name, SpanKind::Item);
                    }
                    Some(MARKER_KEY_ITEM) => {
                        let name = params
                            .key_items
                            .get(slot)
                            .copied()
                            .flatten()
                            .unwrap_or_default();
                        c.push_span(name, SpanKind::KeyItem);
                    }
                    Some(_) => {
                        let n = params.numbers.get(slot).copied().unwrap_or(0);
                        c.push_text(&n.to_string());
                    }
                    None => {}
                }
                i += tag.len;
                continue;
            }
            i += 1;
            continue;
        }

        if matches!(b, CC_NUM | CC_NUM2) {
            let slot = entry.get(i + 1).copied().unwrap_or(0) as usize;
            let n = params.numbers.get(slot).copied().unwrap_or(0);
            c.push_text(&n.to_string());
            i += 2;
            continue;
        }

        if b == CC_TEXT_PARAM {
            let slot = entry.get(i + 1).copied().unwrap_or(0) as usize;
            let s = params
                .strings
                .get(slot)
                .copied()
                .flatten()
                .unwrap_or_default()
                .to_string();
            c.push_text(&s);
            i += 2;
            continue;
        }

        if b == ALT_OPEN && c.alt != Alt::None {
            if let Some((first, second, after)) = split_alternative(&entry[i..]) {
                let keep_first = match c.alt {
                    Alt::Article => params.target_article,
                    Alt::Plural(slot) => params.numbers.get(slot).copied().unwrap_or(0) == 1,
                    Alt::None => true,
                };
                let branch = if keep_first { first } else { second }.to_string();
                c.push_text(&branch);
                c.alt = Alt::None;
                i += after;
                continue;
            }
        }

        if b == CC_NEWLINE {
            c.newline();
        } else if PRINTABLE.contains(&b) {
            let ch = b as char;
            c.push_text(ch.encode_utf8(&mut [0u8; 4]));
        } else if dmsg::is_sjis_lead(b) {
            c.push_text("\u{FFFD}"); // cp932 double-byte run not yet mapped
            i += 1;
        }
        i += 1;
    }

    SysMesLine {
        log_mode,
        lines: c.finish(),
    }
}

/// The item name the next inline item tag will substitute — the article slot
/// sits ahead of its item, so choosing "a" or "an" means looking forward.
fn next_item_name<'a>(entry: &[u8], from: usize, params: &SysMesParams<'a>) -> Option<&'a str> {
    let mut i = from;
    while i < entry.len() {
        if entry[i] == CC_INLINE_TAG {
            if let Some(tag) = parse_inline_tag(entry, i) {
                if tag.marker == Some(MARKER_ITEM) {
                    return params.items.get(tag.param as usize).copied().flatten();
                }
                i += tag.len;
                continue;
            }
        }
        i += 1;
    }
    None
}

/// Retail writes "a lizard tail" and "a pair of bounding boots" — the article
/// follows the item name's leading sound, approximated by its leading letter.
/// An empty name yields "a", matching the entry's fallback spacing.
fn article_for(item_name: &str) -> &'static str {
    const VOWELS: [char; 5] = ['a', 'e', 'i', 'o', 'u'];
    match item_name.chars().next() {
        Some(c) if VOWELS.contains(&c.to_ascii_lowercase()) => "an",
        _ => "a",
    }
}

fn format_gil(amount: i64) -> String {
    let mut grouped = String::new();
    let digits = amount.abs().to_string();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(GIL_GROUP_DIGITS) {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let sign = if amount < 0 { "-" } else { "" };
    format!("{sign}{grouped} {GIL_UNIT}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(text: &str) -> Vec<Span> {
        vec![Span {
            text: text.to_string(),
            kind: SpanKind::Text,
        }]
    }

    #[test]
    fn article_follows_the_leading_letter() {
        assert_eq!(article_for("lizard tail"), "a");
        assert_eq!(article_for("pair of bounding boots"), "a");
        assert_eq!(article_for("ingot"), "an");
        assert_eq!(article_for("Emperor's Ring"), "an");
        assert_eq!(article_for(""), "a");
    }

    #[test]
    fn gil_is_grouped_in_thousands() {
        assert_eq!(format_gil(0), "0 gil");
        assert_eq!(format_gil(42), "42 gil");
        assert_eq!(format_gil(1_200), "1,200 gil");
        assert_eq!(format_gil(1_234_567), "1,234,567 gil");
    }

    /// `1f 79` log mode, literal text, `1c 02` text param, `0a 00` number.
    #[test]
    fn log_mode_and_params_substitute() {
        let entry = b"\x1fy\x1c\x02 rolls \x0a\x00!";
        let mut p = SysMesParams::default();
        p.strings[2] = Some("Daisy");
        p.numbers[0] = 7;
        let line = compose(entry, &p);
        assert_eq!(line.log_mode, Some(0x79));
        assert_eq!(line.to_plain(), "Daisy rolls 7!");
    }

    #[test]
    fn an_item_tag_becomes_its_own_span() {
        // `01 05 27 82 80 80 80` — item name from parameter 0.
        let entry = b"\x1fyYou obtain \x01\x01\x01 \x01\x05'\x82\x80\x80\x80.";
        let mut p = SysMesParams::default();
        p.items[0] = Some("lizard tail");
        let line = compose(entry, &p);
        assert_eq!(line.lines.len(), 1);
        assert_eq!(
            line.lines[0],
            vec![
                Span {
                    text: "You obtain a ".into(),
                    kind: SpanKind::Text
                },
                Span {
                    text: "lizard tail".into(),
                    kind: SpanKind::Item
                },
                Span {
                    text: ".".into(),
                    kind: SpanKind::Text
                },
            ],
            "the item name must be isolated so it can take retail's green"
        );
    }

    #[test]
    fn article_looks_ahead_to_the_item_it_introduces() {
        let entry = b"You find \x01\x01\x01 \x01\x05'\x82\x80\x80\x80.";
        let mut p = SysMesParams::default();
        p.items[0] = Some("ingot");
        assert_eq!(compose(entry, &p).to_plain(), "You find an ingot.");
    }

    #[test]
    fn the_article_alternative_drops_for_a_named_entity() {
        let entry = b"on \x7f\x88\x01[the /]\x01\x01\x11.";
        let mut p = SysMesParams {
            target_name: Some("Leaping Lizzy"),
            target_article: false,
            ..Default::default()
        };
        assert_eq!(compose(entry, &p).to_plain(), "on Leaping Lizzy.");
        p.target_article = true;
        p.target_name = Some("Rock Lizard");
        assert_eq!(compose(entry, &p).to_plain(), "on the Rock Lizard.");
    }

    #[test]
    fn plural_alternative_follows_its_numeric_parameter() {
        let entry = b"\x12\x00 \x7f\x86\x00[second/seconds] left";
        let mut p = SysMesParams::default();
        p.numbers[0] = 1;
        assert_eq!(compose(entry, &p).to_plain(), "1 second left");
        p.numbers[0] = 9;
        assert_eq!(compose(entry, &p).to_plain(), "9 seconds left");
    }

    #[test]
    fn a_newline_starts_a_second_log_line() {
        let entry = b"\x1f{first\x07second";
        let line = compose(entry, &SysMesParams::default());
        assert_eq!(line.lines, vec![spans("first"), spans("second")]);
        assert_eq!(line.to_plain(), "first\nsecond");
    }

    #[test]
    fn capitalize_code_uppercases_the_next_substitution() {
        let entry = b"\x7f\x80\x01\x01\x05&\x82\x80\x80\x80 lost.";
        let mut p = SysMesParams::default();
        p.items[0] = Some("lizard tail");
        assert_eq!(compose(entry, &p).to_plain(), "Lizard tail lost.");
    }

    #[test]
    fn gil_slot_renders_with_its_unit() {
        let entry = b"\x1f\x7f\x1c\x00 obtains \x7f\xb4\x00.";
        let mut p = SysMesParams::default();
        p.strings[0] = Some("Daisy");
        p.numbers[0] = 1_200;
        let line = compose(entry, &p);
        assert_eq!(line.log_mode, Some(0x7f));
        assert_eq!(line.to_plain(), "Daisy obtains 1,200 gil.");
    }

    #[test]
    fn the_terminator_stops_composition() {
        let entry = b"done.\x7f1\x00\x07trailing";
        assert_eq!(compose(entry, &SysMesParams::default()).to_plain(), "done.");
    }

    #[test]
    fn missing_parameters_render_empty_rather_than_panicking() {
        let entry = b"\x1fy\x1c\x07 finds \x01\x05'\x82\x87\x80\x80 at \x0a\x07.";
        let line = compose(entry, &SysMesParams::default());
        assert_eq!(line.to_plain(), " finds  at 0.");
    }
}
