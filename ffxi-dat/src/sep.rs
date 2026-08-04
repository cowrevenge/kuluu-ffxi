use crate::{DatError, Result};

// research/XIClient/src/XIClient/include/Resource/Derived/CYySepRes.h:9-13
// `struct SepHeader { char SepTag[8]; int FileID; unsigned int field_3C; }` — the
// tag is never read back, FileID is the se id and field_3C carries the flags.
const SE_ID_OFFSET: usize = 8;
const FLAGS_OFFSET: usize = 0x0C;

// research/XIClient/src/XIClient/source/World/Generator/Effects/CYySoundElem.cpp:325-345
// `IsNever()` == `field_3C & 0x80000000`, which is what makes a cue a loop rather than
// a one-shot: CYyGenerator.cpp:1169-1171 unlinks the generator when it is set.
const SEP_FLAG_LOOP: u32 = 0x8000_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sep {
    pub name: [u8; 4],

    pub se_id: u32,

    pub flags: u32,
}

impl Sep {
    pub fn parse(name: [u8; 4], body: &[u8]) -> Result<Self> {
        let Some(id) = body.get(SE_ID_OFFSET..SE_ID_OFFSET + 4) else {
            return Err(DatError::TruncatedChunk {
                offset: 0,
                needed: SE_ID_OFFSET + 4,
                available: body.len(),
            });
        };
        let se_id = u32::from_le_bytes([id[0], id[1], id[2], id[3]]);
        let flags = body
            .get(FLAGS_OFFSET..FLAGS_OFFSET + 4)
            .map(|f| u32::from_le_bytes([f[0], f[1], f[2], f[3]]))
            .unwrap_or(0);
        Ok(Self { name, se_id, flags })
    }

    pub fn loops(&self) -> bool {
        self.flags & SEP_FLAG_LOOP != 0
    }

    pub fn relative_path(&self) -> (String, String) {
        (
            format!("se{:03}", self.se_id / 1000),
            format!("se{:06}.spw", self.se_id),
        )
    }
}

// research/XIClient/src/XIClient/source/Resource/Derived/CYySepRes.cpp:183-207
// `CheckFourCC` plus FileResource.cpp:630-641 `GetActivateTime` as one predicate: four
// ASCII digits with the HHMM tens digits bounded, then hour = n1 + 10*n0 and
// minute = n3 + 10*n2. Names failing it are not time buckets at all — they are the
// payloads of the sibling sound generators (`thnd`, `2107`), and 125 shipped Seps are
// labelled with their own se id (`1087`, `8071`), which the tens bounds reject.
// The `extr`-parent half of CheckFourCC needs the parent name and stays with the caller.
pub fn activate_time_minutes(name: &[u8; 4]) -> Option<u32> {
    if !name.iter().all(u8::is_ascii_digit) {
        return None;
    }
    if name[0] > b'1' || name[1] > b'9' || name[2] > b'5' || name[3] > b'9' {
        return None;
    }
    let digit = |i: usize| (name[i] - b'0') as u32;
    Some((digit(1) + 10 * digit(0)) * 60 + digit(3) + 10 * digit(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_se_id_at_offset_8() {
        let mut body = vec![0u8; 16];
        body[8..12].copy_from_slice(&12345u32.to_le_bytes());
        let s = Sep::parse(*b"snd0", &body).unwrap();
        assert_eq!(s.se_id, 12345);
        assert_eq!(s.name, *b"snd0");
        assert_eq!(
            s.relative_path(),
            ("se012".to_string(), "se012345.spw".to_string())
        );
    }

    // The loop bit lives past the 12 bytes the se id needs, and synthetic 12-byte bodies
    // are still built by callers, so a short body must read as a one-shot rather than
    // failing the parse.
    #[test]
    fn loop_flag_reads_from_field_3c_and_tolerates_a_short_body() {
        let mut body = vec![0u8; 16];
        body[FLAGS_OFFSET..FLAGS_OFFSET + 4].copy_from_slice(&0x8000_0104u32.to_le_bytes());
        assert!(Sep::parse(*b"2024", &body).unwrap().loops());

        body[FLAGS_OFFSET..FLAGS_OFFSET + 4].copy_from_slice(&0x0000_0048u32.to_le_bytes());
        assert!(!Sep::parse(*b"2081", &body).unwrap().loops());

        let short = vec![0u8; 12];
        let s = Sep::parse(*b"se01", &short).unwrap();
        assert_eq!(s.flags, 0);
        assert!(!s.loops());
    }

    #[test]
    fn rejects_short_body() {
        let body = vec![0u8; 4];
        assert!(matches!(
            Sep::parse(*b"abcd", &body),
            Err(DatError::TruncatedChunk {
                needed: 12,
                available: 4,
                ..
            })
        ));
    }

    #[test]
    fn se_zero_resolves_to_se000() {
        let mut body = vec![0u8; 12];
        body[8..12].copy_from_slice(&0u32.to_le_bytes());
        let s = Sep::parse(*b"zero", &body).unwrap();
        assert_eq!(
            s.relative_path(),
            ("se000".to_string(), "se000000.spw".to_string())
        );
    }

    // Transcribes CheckFourCC's truth table. The rejected rows are all real shipped Sep
    // names: `2020`/`2100` are one-shot thunderclaps under weat/thdr that a naive
    // "four digits" rule would mistake for 20:20 and 21:00 time buckets, and `1087`/
    // `8071` are Seps labelled with their own se id.
    #[test]
    fn activate_time_accepts_only_retails_hhmm_domain() {
        assert_eq!(activate_time_minutes(b"0000"), Some(0));
        assert_eq!(activate_time_minutes(b"0600"), Some(360));
        assert_eq!(activate_time_minutes(b"1138"), Some(11 * 60 + 38));
        assert_eq!(activate_time_minutes(b"1959"), Some(19 * 60 + 59));
        assert_eq!(activate_time_minutes(b"0430"), Some(4 * 60 + 30));

        for reject in [
            b"2000", b"2020", b"2100", b"2309", b"1087", b"8071", b"thnd", b"torn", b"se01",
        ] {
            assert_eq!(
                activate_time_minutes(reject),
                None,
                "{}",
                String::from_utf8_lossy(reject)
            );
        }
    }
}
