use super::*;

#[derive(Debug, Clone, Copy)]
pub struct MagicData<'a> {
    pub bitmap: &'a [u8; MAGIC_DATA_SIZE],
}

pub const MAGIC_DATA_SIZE: usize = 128;

impl<'a> MagicData<'a> {
    pub const SIZE: usize = MAGIC_DATA_SIZE;

    pub const SPELL_ID_LIMIT: usize = Self::SIZE * 8;
    pub fn decode(body: &'a [u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::SIZE {
            return Err(DecodeError::Truncated(Self::SIZE, body.len()));
        }
        let bitmap: &[u8; MAGIC_DATA_SIZE] = body[..Self::SIZE].try_into().unwrap();
        Ok(Self { bitmap })
    }

    pub fn known_ids(&self) -> Vec<u16> {
        collect_set_bits(self.bitmap)
    }
    pub fn is_known(&self, id: u16) -> bool {
        let idx = id as usize;
        if idx >= Self::SPELL_ID_LIMIT {
            return false;
        }
        self.bitmap[idx >> 3] & (1 << (idx & 7)) != 0
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CommandData<'a> {
    pub weapon_skills: &'a [u8; 64],
    pub job_abilities: &'a [u8; 64],
    pub pet_abilities: &'a [u8; 64],
    pub traits: &'a [u8; 32],
}

impl<'a> CommandData<'a> {
    pub const SIZE: usize = 64 + 64 + 64 + 32;
    pub fn decode(body: &'a [u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::SIZE {
            return Err(DecodeError::Truncated(Self::SIZE, body.len()));
        }
        Ok(Self {
            weapon_skills: body[0..64].try_into().unwrap(),
            job_abilities: body[64..128].try_into().unwrap(),
            pet_abilities: body[128..192].try_into().unwrap(),
            traits: body[192..224].try_into().unwrap(),
        })
    }
}

pub fn collect_set_bits(bitmap: &[u8]) -> Vec<u16> {
    let mut out = Vec::new();
    for (byte_idx, byte) in bitmap.iter().enumerate() {
        if *byte == 0 {
            continue;
        }
        for bit in 0..8 {
            if byte & (1 << bit) != 0 {
                out.push((byte_idx * 8 + bit) as u16);
            }
        }
    }
    out
}
