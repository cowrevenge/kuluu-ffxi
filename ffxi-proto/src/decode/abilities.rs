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

#[cfg(test)]
mod magic_data_tests {
    use super::*;

    #[test]
    fn magic_data_known_ids_picks_set_bits() {
        let mut buf = [0u8; MagicData::SIZE];

        buf[0] = 0b1000_0001;
        buf[1] = 0b0000_0001;
        buf[2] = 0b0000_0010;
        buf[127] = 0b1000_0000;
        let m = MagicData::decode(&buf).unwrap();
        assert_eq!(m.known_ids(), vec![0, 7, 8, 17, 1023]);
        assert!(m.is_known(0));
        assert!(m.is_known(7));
        assert!(m.is_known(1023));
        assert!(!m.is_known(1));

        assert!(!m.is_known(u16::MAX));
    }

    #[test]
    fn magic_data_truncated_returns_err() {
        let buf = [0u8; MagicData::SIZE - 1];
        assert!(matches!(
            MagicData::decode(&buf),
            Err(DecodeError::Truncated(MagicData::SIZE, n)) if n == MagicData::SIZE - 1
        ));
    }
}

#[cfg(test)]
mod command_data_tests {
    use super::*;

    #[test]
    fn command_data_splits_into_four_bitsets() {
        let mut buf = [0u8; CommandData::SIZE];

        buf[0] = 0xA1;
        buf[64] = 0xA2;
        buf[128] = 0xA3;
        buf[192] = 0xA4;
        let c = CommandData::decode(&buf).unwrap();
        assert_eq!(c.weapon_skills[0], 0xA1);
        assert_eq!(c.job_abilities[0], 0xA2);
        assert_eq!(c.pet_abilities[0], 0xA3);
        assert_eq!(c.traits[0], 0xA4);

        assert_eq!(c.weapon_skills.len(), 64);
        assert_eq!(c.job_abilities.len(), 64);
        assert_eq!(c.pet_abilities.len(), 64);
        assert_eq!(c.traits.len(), 32);
    }

    #[test]
    fn command_data_truncated_returns_err() {
        let buf = [0u8; CommandData::SIZE - 1];
        assert!(matches!(
            CommandData::decode(&buf),
            Err(DecodeError::Truncated(CommandData::SIZE, n)) if n == CommandData::SIZE - 1
        ));
    }
}
