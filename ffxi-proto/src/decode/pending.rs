use super::DecodeError;

/// One s2c 0x05C `GP_SERV_PENDINGNUM`: int32 num[8] that the client copies
/// into its event Work_Zone buffer starting at index 2, where the event system
/// reads them as loop conditions (research/XiPackets/world/server/0x005C).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingNum {
    pub num: [i32; 8],
}

impl PendingNum {
    /// Body size after the 4-byte sub-header (the packet is 0x24 on the wire).
    pub(crate) const SIZE: usize = 8 * std::mem::size_of::<i32>();

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::SIZE {
            return Err(DecodeError::Truncated(Self::SIZE, body.len()));
        }
        let mut num = [0i32; 8];
        for (slot, chunk) in num
            .iter_mut()
            .zip(body.chunks_exact(std::mem::size_of::<i32>()))
        {
            *slot = i32::from_le_bytes(chunk.try_into().unwrap());
        }
        Ok(Self { num })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_eight_little_endian_ints() {
        let mut body = [0u8; PendingNum::SIZE];
        for (i, slot) in body.chunks_exact_mut(4).enumerate() {
            slot.copy_from_slice(&(i as i32 * 1000 - 500).to_le_bytes());
        }
        let decoded = PendingNum::decode(&body).unwrap();
        for (i, value) in decoded.num.iter().enumerate() {
            assert_eq!(*value, i as i32 * 1000 - 500);
        }
    }

    #[test]
    fn rejects_a_truncated_body() {
        assert!(matches!(
            PendingNum::decode(&[0; PendingNum::SIZE - 1]),
            Err(DecodeError::Truncated(PendingNum::SIZE, _))
        ));
    }
}
