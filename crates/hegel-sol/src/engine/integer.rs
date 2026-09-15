use revm::primitives::{I256, U256};

pub fn u256_to_le_signed(value: U256) -> [u8; 33] {
    let mut out = [0; 33];
    out[..32].copy_from_slice(&value.to_le_bytes::<32>());
    out
}

pub fn u256_from_le_signed(value: &[u8]) -> Option<U256> {
    if value.is_empty() || value.len() > 33 || (value.len() == 33 && value[32] != 0) {
        return None;
    }
    let mut bytes = [0; 32];
    bytes[..value.len().min(32)].copy_from_slice(&value[..value.len().min(32)]);
    Some(U256::from_le_bytes(bytes))
}

pub fn i256_to_le_signed(value: I256) -> [u8; 33] {
    let bits = value.into_raw().to_le_bytes::<32>();
    let mut out = [if value.is_negative() { 0xff } else { 0 }; 33];
    out[..32].copy_from_slice(&bits);
    out
}

pub fn i256_from_le_signed(value: &[u8]) -> Option<I256> {
    if value.is_empty() || value.len() > 33 {
        return None;
    }
    let negative = value.last().is_some_and(|byte| byte & 0x80 != 0);
    if value.len() == 33 && value[32] != if negative { 0xff } else { 0 } {
        return None;
    }
    let mut bytes = [if negative { 0xff } else { 0 }; 32];
    bytes[..value.len().min(32)].copy_from_slice(&value[..value.len().min(32)]);
    Some(I256::from_raw(U256::from_le_bytes(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsigned_round_trips() {
        for value in [U256::ZERO, U256::from(1), U256::from(1) << 255, U256::MAX] {
            assert_eq!(u256_from_le_signed(&u256_to_le_signed(value)), Some(value));
        }
    }

    #[test]
    fn signed_round_trips() {
        for value in [
            I256::ZERO,
            I256::try_from(1).unwrap(),
            I256::MINUS_ONE,
            I256::MIN,
        ] {
            assert_eq!(i256_from_le_signed(&i256_to_le_signed(value)), Some(value));
        }
    }
}
