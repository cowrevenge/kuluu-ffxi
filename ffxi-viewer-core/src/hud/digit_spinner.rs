//! Retail AH "Price Set" digit spinner: `All ◄ [0] G ▶` over `/999,999,999 G`
//! (.agents/skills/retail-observe/references/auction-house.md). Left/Right move
//! the active digit column ("All" = the whole-value column at the far left),
//! Up/Down step the active digit by its place value. Pure logic (no Bevy) so
//! bazaar/delivery gil entry can converge on it later.

/// Digit columns rendered/steppable: the AH price validator caps at
/// 999,999,999 (GP_CLI_COMMAND_AUC::validate, ffxi_proto::decode::auction::
/// AUCTION_PRICE_MAX), i.e. nine decimal digits.
pub const PRICE_DIGITS: u32 = 9;

/// The active column: a decimal place, or the whole-value "All" column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpinnerColumn {
    All,
    /// `Digit(p)` is the 10^p place; 0 = ones.
    Digit(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigitSpinner {
    pub value: u32,
    pub cap: u32,
    pub column: SpinnerColumn,
    /// Bitmask of 10^p places the user has stepped (retail tints just-edited
    /// digits orange).
    pub edited: u16,
}

fn pow10(p: u32) -> u32 {
    10u32.saturating_pow(p)
}

/// Decimal digit count of `n` (1 for 0).
pub fn digit_count(n: u32) -> u32 {
    let mut count = 1;
    let mut rest = n / 10;
    while rest > 0 {
        count += 1;
        rest /= 10;
    }
    count
}

impl DigitSpinner {
    /// A spinner over `0..=cap`, parked on the ones digit at 0 (retail opens
    /// showing `[0]`).
    pub fn new(cap: u32) -> Self {
        Self {
            value: 0,
            cap,
            column: SpinnerColumn::Digit(0),
            edited: 0,
        }
    }

    /// A spinner re-opened at a previously entered value (backing out of a
    /// confirm returns to Price Set with the price intact).
    pub fn with_value(cap: u32, value: u32) -> Self {
        Self {
            value: value.min(cap),
            ..Self::new(cap)
        }
    }

    /// Highest digit column this spinner offers (bounded by the cap's width).
    fn max_power(&self) -> u32 {
        digit_count(self.cap).min(PRICE_DIGITS) - 1
    }

    /// `◀`: toward higher place values, ending on the All column.
    pub fn left(&mut self) {
        self.column = match self.column {
            SpinnerColumn::All => SpinnerColumn::All,
            SpinnerColumn::Digit(p) if p >= self.max_power() => SpinnerColumn::All,
            SpinnerColumn::Digit(p) => SpinnerColumn::Digit(p + 1),
        };
    }

    /// `▶`: toward the ones digit.
    pub fn right(&mut self) {
        self.column = match self.column {
            SpinnerColumn::All => SpinnerColumn::Digit(self.max_power()),
            SpinnerColumn::Digit(0) => SpinnerColumn::Digit(0),
            SpinnerColumn::Digit(p) => SpinnerColumn::Digit(p - 1),
        };
    }

    /// `▲ +`: step the active digit up (no carry into the next place; All
    /// jumps to the cap), clamped to the cap.
    pub fn up(&mut self) {
        match self.column {
            SpinnerColumn::All => self.value = self.cap,
            SpinnerColumn::Digit(p) => {
                if self.digit_at(p) < 9 {
                    self.value = self.value.saturating_add(pow10(p)).min(self.cap);
                }
                self.edited |= 1 << p;
            }
        }
    }

    /// `▼ −`: step the active digit down (a 0 digit stays 0 — no borrow from
    /// the next place; All resets to 0).
    pub fn down(&mut self) {
        match self.column {
            SpinnerColumn::All => self.value = 0,
            SpinnerColumn::Digit(p) => {
                if self.digit_at(p) > 0 {
                    self.value -= pow10(p);
                }
                self.edited |= 1 << p;
            }
        }
    }

    /// Digit at place `p` of the current value.
    pub fn digit_at(&self, p: u32) -> u32 {
        (self.value / pow10(p)) % 10
    }

    /// The columns to draw, most significant first: enough places for the
    /// current value and to keep the active column visible.
    pub fn visible_powers(&self) -> impl DoubleEndedIterator<Item = u32> {
        let need = match self.column {
            SpinnerColumn::All => digit_count(self.value),
            SpinnerColumn::Digit(p) => digit_count(self.value).max(p + 1),
        };
        (0..need).rev()
    }
}

/// Comma-grouped gil amount, no suffix: `80147` → `"80,147"`.
pub fn format_gil(n: u32) -> String {
    let digits = n.to_string();
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
    fn opens_on_ones_at_zero() {
        let s = DigitSpinner::new(999_999_999);
        assert_eq!(s.value, 0);
        assert_eq!(s.column, SpinnerColumn::Digit(0));
    }

    #[test]
    fn digit_steps_add_place_value_and_clamp() {
        let mut s = DigitSpinner::new(999_999_999);
        s.up();
        assert_eq!(s.value, 1);
        s.left();
        s.left();
        s.up();
        assert_eq!(s.value, 101, "hundreds column steps by 100");
        s.down();
        s.down();
        assert_eq!(s.value, 1, "clamps at the subtraction floor");
        assert_eq!(s.edited & 0b101, 0b101, "ones + hundreds marked edited");
    }

    #[test]
    fn cap_clamps_up_steps() {
        let mut s = DigitSpinner::new(1_180);
        s.left();
        s.left();
        s.left();
        s.up();
        assert_eq!(s.value, 1_000);
        s.up();
        assert_eq!(s.value, 1_180, "step past the cap clamps to it");
    }

    #[test]
    fn column_walk_ends_on_all_and_returns() {
        let mut s = DigitSpinner::new(80_121);
        for _ in 0..10 {
            s.left();
        }
        assert_eq!(s.column, SpinnerColumn::All, "left walk stops at All");
        s.up();
        assert_eq!(s.value, 80_121, "All + up selects the whole cap");
        s.down();
        assert_eq!(s.value, 0);
        s.right();
        assert_eq!(
            s.column,
            SpinnerColumn::Digit(4),
            "right off All lands on the cap's top digit"
        );
        for _ in 0..10 {
            s.right();
        }
        assert_eq!(
            s.column,
            SpinnerColumn::Digit(0),
            "right walk stops at ones"
        );
    }

    #[test]
    fn visible_powers_cover_value_and_active_column() {
        let mut s = DigitSpinner::new(999_999_999);
        assert_eq!(s.visible_powers().collect::<Vec<_>>(), vec![0]);
        s.left();
        s.left();
        assert_eq!(s.visible_powers().collect::<Vec<_>>(), vec![2, 1, 0]);
        s.up();
        assert_eq!(s.value, 100);
        assert_eq!(s.digit_at(2), 1);
    }

    #[test]
    fn gil_formats_comma_grouped() {
        assert_eq!(format_gil(0), "0");
        assert_eq!(format_gil(999), "999");
        assert_eq!(format_gil(1_180), "1,180");
        assert_eq!(format_gil(80_147), "80,147");
        assert_eq!(format_gil(999_999_999), "999,999,999");
    }
}
