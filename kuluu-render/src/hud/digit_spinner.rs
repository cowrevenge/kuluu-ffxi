//! .agents/skills/retail-observe/references/auction-house.md Price Set

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
    pub min: u32,
    pub column: SpinnerColumn,
    /// Whether the whole-value "All" column is offered. Retail's Price Set has
    /// one; a stack quantity does not need it, because stepping the top digit
    /// already saturates at the cap.
    pub all_column: bool,
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
            min: 0,
            column: SpinnerColumn::Digit(0),
            all_column: true,
            edited: 0,
        }
    }

    /// A stack-quantity picker over `1..=cap`, digits only.
    pub fn item(cap: u32) -> Self {
        Self {
            min: 1,
            all_column: false,
            ..Self::with_value(cap.max(1), 1)
        }
    }

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

    /// "All": select the whole amount, as [`crate::hud::spinner::Spinner::set_all`]
    /// does for the arrow-style pickers.
    pub fn set_all(&mut self) {
        self.value = self.cap;
    }

    /// Start the amount over at the picker's floor.
    pub fn set_min(&mut self) {
        self.value = self.min;
    }

    /// `◀`: toward higher place values, ending on the All column where one is
    /// offered. Without that column, stepping off the top instead takes the
    /// whole amount — the answer All would have given.
    pub fn left(&mut self) {
        self.column = match self.column {
            SpinnerColumn::All => SpinnerColumn::All,
            SpinnerColumn::Digit(p) if p >= self.max_power() => {
                if self.all_column {
                    SpinnerColumn::All
                } else {
                    self.set_all();
                    SpinnerColumn::Digit(p)
                }
            }
            SpinnerColumn::Digit(p) => SpinnerColumn::Digit(p + 1),
        };
    }

    /// `▶`: toward the ones digit, and off the end back to the minimum, so the
    /// same axis that reaches the whole amount also starts over.
    pub fn right(&mut self) {
        self.column = match self.column {
            SpinnerColumn::All => SpinnerColumn::Digit(self.max_power()),
            SpinnerColumn::Digit(0) => {
                if !self.all_column {
                    self.set_min();
                }
                SpinnerColumn::Digit(0)
            }
            SpinnerColumn::Digit(p) => SpinnerColumn::Digit(p - 1),
        };
    }

    /// `▲ +`: add the active place value (carrying into higher digits),
    /// clamped to the cap; All jumps to the cap.
    pub fn up(&mut self) {
        match self.column {
            SpinnerColumn::All => self.set_all(),
            SpinnerColumn::Digit(p) => {
                self.value = self.value.saturating_add(pow10(p)).min(self.cap);
                self.edited |= 1 << p;
            }
        }
    }

    /// `▼ −`: subtract the active place value (borrowing from higher digits —
    /// 600 on the tens steps to 590), bounded by the minimum.
    pub fn down(&mut self) {
        match self.column {
            SpinnerColumn::All => self.set_min(),
            SpinnerColumn::Digit(p) => {
                self.value = self.value.saturating_sub(pow10(p)).max(self.min);
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
    /// Every column this spinner offers, most significant first.
    pub fn columns(&self) -> impl Iterator<Item = SpinnerColumn> {
        self.all_column
            .then_some(SpinnerColumn::All)
            .into_iter()
            .chain((0..=self.max_power()).rev().map(SpinnerColumn::Digit))
    }

    pub fn visible_powers(&self) -> impl DoubleEndedIterator<Item = u32> {
        let need = match self.column {
            SpinnerColumn::All => digit_count(self.value),
            SpinnerColumn::Digit(p) => digit_count(self.value).max(p + 1),
        };
        (0..need).rev()
    }
}

pub fn column_style(
    spinner: &DigitSpinner,
    column: SpinnerColumn,
) -> (String, bevy::prelude::Color, bevy::prelude::Color) {
    use crate::hud::item_ui::theme;
    use bevy::prelude::Color;
    match column {
        SpinnerColumn::All if !spinner.all_column => (String::new(), theme::TEXT, Color::NONE),
        SpinnerColumn::All => (
            "All ".into(),
            if spinner.column == column {
                theme::CURSOR
            } else {
                theme::TEXT
            },
            Color::NONE,
        ),
        SpinnerColumn::Digit(power) => {
            if !spinner.visible_powers().any(|p| p == power) {
                return (String::new(), theme::TEXT, Color::NONE);
            }
            let active = spinner.column == column;
            (
                spinner.digit_at(power).to_string(),
                if active {
                    Color::WHITE
                } else if spinner.edited & (1 << power) != 0 {
                    SPINNER_EDITED
                } else {
                    theme::TEXT
                },
                if active {
                    SPINNER_ACTIVE_BG
                } else {
                    Color::NONE
                },
            )
        }
    }
}

// Approximate the active and edited tints in the auction-house.md recording.
const SPINNER_ACTIVE_BG: bevy::prelude::Color = bevy::prelude::Color::srgba(0.85, 0.25, 0.35, 0.85);
const SPINNER_EDITED: bevy::prelude::Color = bevy::prelude::Color::srgb(1.0, 0.62, 0.25);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shop_quantity_bounds_apply_to_every_digit() {
        let mut spinner = DigitSpinner::item(12);
        spinner.down();
        assert_eq!(spinner.value, 1, "a quantity floors at one, not zero");
        spinner.left();
        spinner.up();
        assert_eq!(spinner.value, 11);
        spinner.up();
        assert_eq!(spinner.value, 12, "the tens digit saturates at the stack");
        spinner.down();
        assert_eq!(spinner.value, 2);
    }

    /// A stack quantity offers digits only: stepping the top digit already
    /// saturates at the cap, so a separate whole-value column would be a second
    /// way to say the same thing.
    #[test]
    fn a_quantity_picker_has_no_all_column() {
        let mut spinner = DigitSpinner::item(12);
        assert_eq!(
            spinner.columns().collect::<Vec<_>>(),
            vec![SpinnerColumn::Digit(1), SpinnerColumn::Digit(0)]
        );

        spinner.left();
        assert_eq!(
            spinner.column,
            SpinnerColumn::Digit(1),
            "left stops on the top digit instead of stepping onto All"
        );
        assert_eq!(column_style(&spinner, SpinnerColumn::All).0, "");

        spinner.up();
        assert_eq!(spinner.value, 11);
        spinner.up();
        assert_eq!(
            spinner.value, 12,
            "the top digit still reaches the whole stack"
        );
    }

    /// Without an All column the ends of the digit row carry the two answers it
    /// would have given: one step off the top takes everything, one step off the
    /// ones starts over.
    #[test]
    fn walking_off_either_end_takes_the_whole_stack_or_starts_over() {
        let mut spinner = DigitSpinner::item(12);
        assert_eq!(spinner.value, 1);

        spinner.left();
        assert_eq!(spinner.value, 1, "the top digit is still a digit");
        spinner.left();
        assert_eq!(spinner.value, 12, "one past the top takes the whole stack");
        assert_eq!(spinner.column, SpinnerColumn::Digit(1));

        spinner.right();
        assert_eq!(spinner.value, 12, "the ones digit is still a digit");
        spinner.right();
        assert_eq!(spinner.value, 1, "one past the ones starts over at the min");
        assert_eq!(spinner.column, SpinnerColumn::Digit(0));
    }

    /// The price picker has an All column to hold those answers, so its ends
    /// stay put.
    #[test]
    fn a_price_pickers_ends_do_not_move_the_value() {
        let mut spinner = DigitSpinner::with_value(crate::hud::auction::PRICE_CAP, 4_200);
        spinner.right();
        assert_eq!(spinner.value, 4_200);
        for _ in 0..PRICE_DIGITS + 2 {
            spinner.left();
        }
        assert_eq!(spinner.column, SpinnerColumn::All);
        assert_eq!(spinner.value, 4_200);
    }

    /// Retail's Price Set does have one
    /// (.agents/skills/retail-observe/references/auction-house.md), so the
    /// auction house keeps it.
    #[test]
    fn a_price_picker_keeps_its_all_column() {
        let mut spinner = DigitSpinner::new(crate::hud::auction::PRICE_CAP);
        assert!(spinner.columns().any(|c| c == SpinnerColumn::All));
        for _ in 0..PRICE_DIGITS + 1 {
            spinner.left();
        }
        assert_eq!(spinner.column, SpinnerColumn::All);
        assert_eq!(column_style(&spinner, SpinnerColumn::All).0, "All ");
    }

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
        assert_eq!(s.value, 0, "subtraction floors at 0, not the digit");
        assert_eq!(s.edited & 0b101, 0b101, "ones + hundreds marked edited");
    }

    #[test]
    fn down_borrows_across_digits() {
        let mut s = DigitSpinner::with_value(999_999_999, 600);
        s.column = SpinnerColumn::Digit(1);
        s.down();
        assert_eq!(s.value, 590, "600 minus a tens step borrows from the 6");
        s.up();
        s.up();
        assert_eq!(s.value, 610, "590 plus two tens steps carries back");
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
}
