//! Gil presentation shared by the session's chat lines and the HUD, so the two
//! cannot drift apart.

/// Retail writes gil with thousands separators everywhere it shows an amount.
pub fn group_digits(value: u32) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_group_into_thousands_like_retail() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1_000), "1,000");
        assert_eq!(group_digits(24_999), "24,999");
        assert_eq!(group_digits(1_389_292), "1,389,292");
        assert_eq!(group_digits(14_000_000), "14,000,000");
    }
}
