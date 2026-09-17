//! `FFXiMain.dll` ships no code on disk: `.text` has a zero raw size and the
//! real bytes sit LZSS-packed in a `POL1` section that the entry-point stub
//! inflates at load. research/xi-tools/docs/ffximain/ffximain.md documents the
//! container and the bit format under `lzss_decompress`.

pub const SECTION_NAME_LEN: usize = 8;

const TEXT_SECTION_NAME: &str = ".text";
const POL1_SECTION_NAME: &str = "POL1";

pub(crate) const PE_E_LFANEW_OFFSET: usize = 0x3C;
pub(crate) const PE_SIGNATURE: &[u8; 4] = b"PE\0\0";
pub(crate) const PE_NUMBER_OF_SECTIONS_OFFSET: usize = 6;
pub(crate) const PE_SIZE_OF_OPTIONAL_HEADER_OFFSET: usize = 20;
/// The optional header follows the `PE\0\0` signature and the COFF header; the
/// section table follows the optional header, whose size the COFF header gives.
pub(crate) const PE_OPTIONAL_HEADER_OFFSET: usize = 24;
pub(crate) const OPTIONAL_HEADER_ENTRY_POINT_OFFSET: usize = 16;
pub(crate) const SECTION_HEADER_SIZE: usize = 40;
pub(crate) const SECTION_VIRTUAL_SIZE_OFFSET: usize = 8;
pub(crate) const SECTION_VIRTUAL_ADDRESS_OFFSET: usize = 12;
pub(crate) const SECTION_SIZE_OF_RAW_DATA_OFFSET: usize = 16;
pub(crate) const SECTION_POINTER_TO_RAW_DATA_OFFSET: usize = 20;

// research/xi-tools/docs/ffximain/ffximain.md `lzss_decompress`: a control byte
// carries eight flags, MSB first. A set flag copies one literal byte; a clear
// one reads a big-endian word whose low twelve bits are the lookback distance
// (zero ends the stream) and whose top nibble is the copy length minus three.
const CONTROL_LITERAL_BIT: u8 = 0x80;
const BACKREF_DISTANCE_MASK: u16 = 0x0FFF;
const BACKREF_LENGTH_SHIFT: u32 = 12;
const BACKREF_MIN_LENGTH: usize = 3;
const BACKREF_WORD_LEN: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Pol1Error {
    #[error("not a PE image: no `PE\\0\\0` signature at e_lfanew")]
    NotPe,

    #[error("PE image has no {name} section")]
    MissingSection { name: &'static str },

    #[error("{name} raw span {start:#x}..{end:#x} is outside the {len:#x}-byte image")]
    SectionOutsideImage {
        name: &'static str,
        start: usize,
        end: usize,
        len: usize,
    },

    #[error("back-reference {distance:#x} bytes back at output offset {at:#x} points before the start of .text")]
    BackReferenceBeforeStart { at: usize, distance: usize },

    #[error("packed stream yielded {produced:#x} of the {expected:#x} bytes .text declares")]
    Truncated { produced: usize, expected: usize },

    #[error("rva {rva:#x} + {len:#x} is outside the unpacked .text at {base_rva:#x} + {size:#x}")]
    RvaOutOfRange {
        rva: u32,
        len: usize,
        base_rva: u32,
        size: usize,
    },
}

pub type Result<T> = std::result::Result<T, Pol1Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeSection {
    pub name: [u8; SECTION_NAME_LEN],
    pub virtual_address: u32,
    pub virtual_size: u32,
    pub pointer_to_raw_data: u32,
    pub size_of_raw_data: u32,
}

/// The unpacked `.text` bytes with the RVA they start at, so a disassembly
/// citation can be read back without the caller tracking the section base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextImage {
    pub base_rva: u32,
    pub bytes: Vec<u8>,
}

impl TextImage {
    pub fn unpack(dll_bytes: &[u8]) -> Result<Self> {
        let text = require_section(dll_bytes, TEXT_SECTION_NAME)?;
        let pol1 = require_section(dll_bytes, POL1_SECTION_NAME)?;
        let start = pol1.pointer_to_raw_data as usize;
        let end = start.saturating_add(pol1.size_of_raw_data as usize);
        let packed = dll_bytes
            .get(start..end)
            .ok_or(Pol1Error::SectionOutsideImage {
                name: POL1_SECTION_NAME,
                start,
                end,
                len: dll_bytes.len(),
            })?;
        Ok(Self {
            base_rva: text.virtual_address,
            bytes: lzss_decompress(packed, text.virtual_size as usize)?,
        })
    }

    pub fn at_rva(&self, rva: u32, len: usize) -> Result<&[u8]> {
        rva.checked_sub(self.base_rva)
            .and_then(|off| {
                let off = off as usize;
                self.bytes.get(off..off.checked_add(len)?)
            })
            .ok_or(Pol1Error::RvaOutOfRange {
                rva,
                len,
                base_rva: self.base_rva,
                size: self.bytes.len(),
            })
    }
}

pub fn unpack_text(dll_bytes: &[u8]) -> Result<Vec<u8>> {
    TextImage::unpack(dll_bytes).map(|image| image.bytes)
}

/// `AddressOfEntryPoint`, which on a packed image is the POL1 unpacker stub.
pub fn entry_point_rva(dll_bytes: &[u8]) -> Option<u32> {
    let pe = pe_signature_offset(dll_bytes)?;
    read_u32_le(
        dll_bytes,
        pe.checked_add(PE_OPTIONAL_HEADER_OFFSET)?
            .checked_add(OPTIONAL_HEADER_ENTRY_POINT_OFFSET)?,
    )
}

pub const fn section_name(name: &[u8]) -> [u8; SECTION_NAME_LEN] {
    let mut padded = [0u8; SECTION_NAME_LEN];
    let mut i = 0;
    while i < name.len() {
        padded[i] = name[i];
        i += 1;
    }
    padded
}

pub fn find_section(dll_bytes: &[u8], name: &[u8; SECTION_NAME_LEN]) -> Option<PeSection> {
    let pe = pe_signature_offset(dll_bytes)?;
    let count = read_u16_le(dll_bytes, pe.checked_add(PE_NUMBER_OF_SECTIONS_OFFSET)?)? as usize;
    let optional = read_u16_le(
        dll_bytes,
        pe.checked_add(PE_SIZE_OF_OPTIONAL_HEADER_OFFSET)?,
    )? as usize;
    let table = pe
        .checked_add(PE_OPTIONAL_HEADER_OFFSET)?
        .checked_add(optional)?;
    for i in 0..count {
        let at = table.checked_add(i.checked_mul(SECTION_HEADER_SIZE)?)?;
        if dll_bytes.get(at..at.checked_add(SECTION_NAME_LEN)?)? != &name[..] {
            continue;
        }
        return Some(PeSection {
            name: *name,
            virtual_size: read_u32_le(dll_bytes, at + SECTION_VIRTUAL_SIZE_OFFSET)?,
            virtual_address: read_u32_le(dll_bytes, at + SECTION_VIRTUAL_ADDRESS_OFFSET)?,
            size_of_raw_data: read_u32_le(dll_bytes, at + SECTION_SIZE_OF_RAW_DATA_OFFSET)?,
            pointer_to_raw_data: read_u32_le(dll_bytes, at + SECTION_POINTER_TO_RAW_DATA_OFFSET)?,
        });
    }
    None
}

fn require_section(dll_bytes: &[u8], name: &'static str) -> Result<PeSection> {
    if pe_signature_offset(dll_bytes).is_none() {
        return Err(Pol1Error::NotPe);
    }
    find_section(dll_bytes, &section_name(name.as_bytes()))
        .ok_or(Pol1Error::MissingSection { name })
}

fn pe_signature_offset(dll_bytes: &[u8]) -> Option<usize> {
    let pe = read_u32_le(dll_bytes, PE_E_LFANEW_OFFSET)? as usize;
    (dll_bytes.get(pe..pe.checked_add(PE_SIGNATURE.len())?)? == PE_SIGNATURE).then_some(pe)
}

fn lzss_decompress(packed: &[u8], unpacked_len: usize) -> Result<Vec<u8>> {
    let mut out: Vec<u8> = Vec::with_capacity(unpacked_len);
    let mut at = 0usize;
    'stream: while at < packed.len() && out.len() < unpacked_len {
        let mut control = packed[at];
        at += 1;
        for _ in 0..u8::BITS {
            if out.len() >= unpacked_len {
                break 'stream;
            }
            let literal = control & CONTROL_LITERAL_BIT != 0;
            control <<= 1;
            if literal {
                let Some(&byte) = packed.get(at) else {
                    break 'stream;
                };
                at += 1;
                out.push(byte);
                continue;
            }
            let Some(word) = packed.get(at..at + BACKREF_WORD_LEN) else {
                break 'stream;
            };
            at += BACKREF_WORD_LEN;
            let word = u16::from_be_bytes([word[0], word[1]]);
            let distance = usize::from(word & BACKREF_DISTANCE_MASK);
            if distance == 0 {
                break 'stream;
            }
            if distance > out.len() {
                return Err(Pol1Error::BackReferenceBeforeStart {
                    at: out.len(),
                    distance,
                });
            }
            let run = (usize::from(word >> BACKREF_LENGTH_SHIFT) + BACKREF_MIN_LENGTH)
                .min(unpacked_len - out.len());
            for _ in 0..run {
                out.push(out[out.len() - distance]);
            }
        }
    }
    if out.len() != unpacked_len {
        return Err(Pol1Error::Truncated {
            produced: out.len(),
            expected: unpacked_len,
        });
    }
    Ok(out)
}

fn read_u16_le(bytes: &[u8], off: usize) -> Option<u16> {
    let b = bytes.get(off..off.checked_add(2)?)?;
    Some(u16::from_le_bytes([b[0], b[1]]))
}

fn read_u32_le(bytes: &[u8], off: usize) -> Option<u32> {
    let b = bytes.get(off..off.checked_add(4)?)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One control byte's worth of flags, MSB first, over `items`: a `Some`
    /// byte is a literal, a `None` is the next back-reference word.
    fn packed(items: &[Option<u8>], backrefs: &[(usize, usize)]) -> Vec<u8> {
        let mut control = 0u8;
        let mut body = Vec::new();
        let mut refs = backrefs.iter();
        for (i, item) in items.iter().enumerate() {
            let bit = CONTROL_LITERAL_BIT >> i;
            match item {
                Some(byte) => {
                    control |= bit;
                    body.push(*byte);
                }
                None => {
                    let (distance, run) = *refs.next().expect("a back-reference per empty slot");
                    let word = ((run - BACKREF_MIN_LENGTH) as u16) << BACKREF_LENGTH_SHIFT
                        | (distance as u16 & BACKREF_DISTANCE_MASK);
                    body.extend_from_slice(&word.to_be_bytes());
                }
            }
        }
        let mut out = vec![control];
        out.append(&mut body);
        out
    }

    #[test]
    fn a_literal_run_then_a_back_reference_repeats_the_run() {
        let stream = packed(
            &[Some(b'p'), Some(b'o'), Some(b'l'), None, Some(b'!')],
            &[(3, 6)],
        );
        assert_eq!(lzss_decompress(&stream, 10).unwrap(), b"polpolpol!");
    }

    #[test]
    fn a_back_reference_reaching_before_the_output_is_rejected() {
        let stream = packed(&[Some(b'a'), None], &[(2, 3)]);
        assert_eq!(
            lzss_decompress(&stream, 4),
            Err(Pol1Error::BackReferenceBeforeStart { at: 1, distance: 2 })
        );
        let unopened = packed(&[None], &[(1, 3)]);
        assert_eq!(
            lzss_decompress(&unopened, 3),
            Err(Pol1Error::BackReferenceBeforeStart { at: 0, distance: 1 })
        );
    }

    #[test]
    fn a_zero_distance_ends_the_stream_and_a_short_stream_is_truncated() {
        let stream = packed(&[Some(b'a'), None, Some(b'b')], &[(0, 3)]);
        assert_eq!(
            lzss_decompress(&stream, 4),
            Err(Pol1Error::Truncated {
                produced: 1,
                expected: 4
            })
        );
        assert_eq!(lzss_decompress(&stream, 1).unwrap(), b"a");
        assert_eq!(
            lzss_decompress(&[], 1),
            Err(Pol1Error::Truncated {
                produced: 0,
                expected: 1
            })
        );
    }

    #[test]
    fn a_run_is_clipped_to_the_declared_length() {
        let stream = packed(&[Some(b'x'), None], &[(1, 18)]);
        assert_eq!(lzss_decompress(&stream, 4).unwrap(), b"xxxx");
    }

    const SYNTHETIC_PE_OFFSET: usize = 0x80;
    /// PE32's optional header size, so the synthetic section table sits past
    /// the entry-point field the way a real image's does.
    const SYNTHETIC_OPTIONAL_HEADER_SIZE: usize = 0xE0;
    const SYNTHETIC_TEXT_RVA: u32 = 0x1000;
    const SYNTHETIC_POL1_RAW: usize = 0x200;
    const SYNTHETIC_ENTRY_RVA: u32 = 0x9000;

    /// A minimal packed image: DOS stub pointer, `PE\0\0`, a COFF header naming
    /// `.text` (raw size 0, as both packed builds ship it) and `POL1`.
    fn synthetic_dll(packed_text: &[u8], unpacked_len: u32) -> Vec<u8> {
        let pe = SYNTHETIC_PE_OFFSET;
        let mut bytes = vec![0u8; SYNTHETIC_POL1_RAW + packed_text.len()];
        bytes[PE_E_LFANEW_OFFSET..PE_E_LFANEW_OFFSET + 4]
            .copy_from_slice(&(pe as u32).to_le_bytes());
        bytes[pe..pe + PE_SIGNATURE.len()].copy_from_slice(PE_SIGNATURE);
        bytes[pe + PE_NUMBER_OF_SECTIONS_OFFSET..pe + PE_NUMBER_OF_SECTIONS_OFFSET + 2]
            .copy_from_slice(&2u16.to_le_bytes());
        bytes[pe + PE_SIZE_OF_OPTIONAL_HEADER_OFFSET..pe + PE_SIZE_OF_OPTIONAL_HEADER_OFFSET + 2]
            .copy_from_slice(&(SYNTHETIC_OPTIONAL_HEADER_SIZE as u16).to_le_bytes());
        bytes[pe + PE_OPTIONAL_HEADER_OFFSET + OPTIONAL_HEADER_ENTRY_POINT_OFFSET
            ..pe + PE_OPTIONAL_HEADER_OFFSET + OPTIONAL_HEADER_ENTRY_POINT_OFFSET + 4]
            .copy_from_slice(&SYNTHETIC_ENTRY_RVA.to_le_bytes());
        let mut write = |at: usize, name: &str, vaddr: u32, vsize: u32, ptr: u32, raw: u32| {
            bytes[at..at + SECTION_NAME_LEN].copy_from_slice(&section_name(name.as_bytes()));
            bytes[at + SECTION_VIRTUAL_SIZE_OFFSET..at + SECTION_VIRTUAL_SIZE_OFFSET + 4]
                .copy_from_slice(&vsize.to_le_bytes());
            bytes[at + SECTION_VIRTUAL_ADDRESS_OFFSET..at + SECTION_VIRTUAL_ADDRESS_OFFSET + 4]
                .copy_from_slice(&vaddr.to_le_bytes());
            bytes[at + SECTION_SIZE_OF_RAW_DATA_OFFSET..at + SECTION_SIZE_OF_RAW_DATA_OFFSET + 4]
                .copy_from_slice(&raw.to_le_bytes());
            bytes[at + SECTION_POINTER_TO_RAW_DATA_OFFSET
                ..at + SECTION_POINTER_TO_RAW_DATA_OFFSET + 4]
                .copy_from_slice(&ptr.to_le_bytes());
        };
        let table = pe + PE_OPTIONAL_HEADER_OFFSET + SYNTHETIC_OPTIONAL_HEADER_SIZE;
        write(
            table,
            TEXT_SECTION_NAME,
            SYNTHETIC_TEXT_RVA,
            unpacked_len,
            0,
            0,
        );
        write(
            table + SECTION_HEADER_SIZE,
            POL1_SECTION_NAME,
            SYNTHETIC_ENTRY_RVA,
            packed_text.len() as u32,
            SYNTHETIC_POL1_RAW as u32,
            packed_text.len() as u32,
        );
        bytes[SYNTHETIC_POL1_RAW..].copy_from_slice(packed_text);
        bytes
    }

    #[test]
    fn unpack_text_inflates_the_pol1_section_into_the_declared_text_length() {
        let stream = packed(&[Some(b'p'), Some(b'o'), Some(b'l'), None], &[(3, 3)]);
        let dll = synthetic_dll(&stream, 6);
        assert_eq!(unpack_text(&dll).unwrap(), b"polpol");
        assert_eq!(entry_point_rva(&dll), Some(SYNTHETIC_ENTRY_RVA));
    }

    #[test]
    fn at_rva_reads_the_unpacked_text_and_rejects_an_address_outside_it() {
        let stream = packed(&[Some(b'p'), Some(b'o'), Some(b'l'), None], &[(3, 3)]);
        let image = TextImage::unpack(&synthetic_dll(&stream, 6)).unwrap();
        assert_eq!(image.at_rva(SYNTHETIC_TEXT_RVA + 3, 3).unwrap(), b"pol");
        assert!(image.at_rva(SYNTHETIC_TEXT_RVA - 1, 1).is_err());
        assert!(image.at_rva(SYNTHETIC_TEXT_RVA + 4, 3).is_err());
    }

    #[test]
    fn a_non_pe_image_is_rejected_without_panicking() {
        assert_eq!(unpack_text(&[]), Err(Pol1Error::NotPe));
        assert_eq!(unpack_text(&[0xFFu8; 0x400]), Err(Pol1Error::NotPe));
        let mut truncated = synthetic_dll(&packed(&[Some(b'a')], &[]), 1);
        truncated.truncate(SYNTHETIC_POL1_RAW);
        assert!(matches!(
            unpack_text(&truncated),
            Err(Pol1Error::SectionOutsideImage { .. })
        ));
    }

    #[test]
    fn an_image_whose_section_table_lacks_pol1_names_the_missing_section() {
        let mut dll = synthetic_dll(&packed(&[Some(b'a')], &[]), 1);
        let pol1_header = SYNTHETIC_PE_OFFSET
            + PE_OPTIONAL_HEADER_OFFSET
            + SYNTHETIC_OPTIONAL_HEADER_SIZE
            + SECTION_HEADER_SIZE;
        dll[pol1_header..pol1_header + SECTION_NAME_LEN].copy_from_slice(&section_name(b".rsrc"));
        assert_eq!(
            unpack_text(&dll),
            Err(Pol1Error::MissingSection {
                name: POL1_SECTION_NAME
            })
        );
    }
}
