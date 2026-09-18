//! The one numeric-amount picker. Retail draws `All <arrow> <digits> <arrow>`
//! for every amount it asks for — an auction price
//! (.agents/skills/retail-observe/references/auction-house.md "Price Set") and
//! a stack quantity
//! (.agents/skills/retail-observe/references/2026-07-17-moghouse-menu.md
//! "Send flow" step 4) are the same control over different bounds. Model here,
//! cell layout here; each screen only places the cells.

use bevy::prelude::Color;
use ffxi_vocab::gil::{group_digits, GROUP_SEPARATOR, GROUP_SIZE};

/// Digit columns rendered/steppable: the AH price validator caps at
/// 999,999,999 (GP_CLI_COMMAND_AUC::validate, ffxi_proto::decode::auction::
/// AUCTION_PRICE_MAX), i.e. nine decimal digits.
pub const PRICE_DIGITS: u32 = 9;

/// Text cells one drawn row needs: every digit place plus the separators
/// between its groups.
pub const SPINNER_CELLS: usize = PRICE_DIGITS as usize + (PRICE_DIGITS as usize - 1) / GROUP_SIZE;

/// The active column: a decimal place, or the whole-value "All" column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpinnerColumn {
    All,
    /// `Digit(p)` is the 10^p place; 0 = ones.
    Digit(u32),
}

/// What the amount counts, which decides the unit drawn after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpinnerUnit {
    Count,
    Gil,
}

impl SpinnerUnit {
    pub fn suffix(self) -> &'static str {
        match self {
            SpinnerUnit::Count => "",
            SpinnerUnit::Gil => GIL_UNIT,
        }
    }
}

/// One text node of a drawn spinner row, in [`slots`] order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpinnerSlot {
    All,
    Cell(usize),
    Suffix,
    Cap,
}

/// Every digit/separator cell of the row, most significant first.
pub fn cells() -> impl Iterator<Item = SpinnerSlot> {
    (0..SPINNER_CELLS).map(SpinnerSlot::Cell)
}

/// The whole row on one line, the retail stack quantity's shape
/// (`All < 1 / 2 >`).
pub fn slots() -> impl Iterator<Item = SpinnerSlot> {
    std::iter::once(SpinnerSlot::All)
        .chain(cells())
        .chain([SpinnerSlot::Cap, SpinnerSlot::Suffix])
}

/// The row minus its cap, for a box too narrow to hold a nine-digit cap beside
/// the amount. That layout draws [`SpinnerSlot::Cap`] on the line below, which
/// is how retail's Price Set fits `/999,999,999 G` under the price
/// (.agents/skills/retail-observe/references/auction-house.md).
pub fn slots_without_cap() -> impl Iterator<Item = SpinnerSlot> {
    std::iter::once(SpinnerSlot::All)
        .chain(cells())
        .chain([SpinnerSlot::Suffix])
}

/// One control, one type size, so the amount reads the same in every window.
pub const SPINNER_TEXT_PX: f32 = 15.0;

/// Spawn one drawn row as a flex row of text nodes, each tagged with the
/// caller's own marker so its update system can fill them from [`slot_style`].
pub fn spawn_row<M: bevy::prelude::Component>(
    parent: &mut bevy::prelude::ChildSpawnerCommands,
    slots: impl Iterator<Item = SpinnerSlot>,
    mark: impl Fn(SpinnerSlot) -> M,
) {
    use bevy::prelude::*;
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|row| {
            for slot in slots {
                row.spawn((
                    mark(slot),
                    Text::new(""),
                    crate::hud::item_ui::text_font(SPINNER_TEXT_PX),
                    TextColor(crate::hud::item_ui::theme::TEXT),
                    BackgroundColor(Color::NONE),
                ));
            }
        });
}

/// What cell `i` draws: a decimal place, or the separator that follows the
/// `above` places to its left.
enum CellKind {
    Digit(u32),
    Separator { above: u32 },
}

fn layout() -> impl Iterator<Item = CellKind> {
    (0..PRICE_DIGITS).rev().flat_map(|p| {
        let group_break = p + 1 < PRICE_DIGITS && (p as usize + 1).is_multiple_of(GROUP_SIZE);
        group_break
            .then_some(CellKind::Separator { above: p + 1 })
            .into_iter()
            .chain(std::iter::once(CellKind::Digit(p)))
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigitSpinner {
    pub value: u32,
    pub cap: u32,
    pub min: u32,
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
            min: 0,
            column: SpinnerColumn::Digit(0),
            edited: 0,
        }
    }

    /// A stack-quantity picker over `1..=cap`, opening on 1.
    pub fn item(cap: u32) -> Self {
        Self {
            min: 1,
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

    /// "All": select the whole amount.
    pub fn set_all(&mut self) {
        self.value = self.cap;
    }

    /// Start the amount over at the picker's floor.
    pub fn set_min(&mut self) {
        self.value = self.min;
    }

    /// The amount to commit.
    pub fn confirm(&self) -> u32 {
        self.value.clamp(self.min, self.cap)
    }

    pub fn is_all(&self) -> bool {
        self.value == self.cap
    }

    /// Toward higher place values, ending on the All column.
    pub fn left(&mut self) {
        self.column = match self.column {
            SpinnerColumn::All => SpinnerColumn::All,
            SpinnerColumn::Digit(p) if p >= self.max_power() => SpinnerColumn::All,
            SpinnerColumn::Digit(p) => SpinnerColumn::Digit(p + 1),
        };
    }

    /// Toward the ones digit.
    pub fn right(&mut self) {
        self.column = match self.column {
            SpinnerColumn::All => SpinnerColumn::Digit(self.max_power()),
            SpinnerColumn::Digit(0) => SpinnerColumn::Digit(0),
            SpinnerColumn::Digit(p) => SpinnerColumn::Digit(p - 1),
        };
    }

    /// Add the active place value (carrying into higher digits), clamped to the
    /// cap; All jumps to the cap.
    pub fn up(&mut self) {
        match self.column {
            SpinnerColumn::All => self.set_all(),
            SpinnerColumn::Digit(p) => {
                self.value = self.value.saturating_add(pow10(p)).min(self.cap);
                self.edited |= 1 << p;
            }
        }
    }

    /// Subtract the active place value (borrowing from higher digits — 600 on
    /// the tens steps to 590), bounded by the minimum.
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

    /// Places wide enough for the current value and to keep the active column
    /// visible, most significant first.
    pub fn visible_powers(&self) -> impl DoubleEndedIterator<Item = u32> {
        let need = match self.column {
            SpinnerColumn::All => digit_count(self.value),
            SpinnerColumn::Digit(p) => digit_count(self.value).max(p + 1),
        };
        (0..need).rev()
    }
}

/// Text, tint and background for one node of the drawn row.
pub fn slot_style(
    spinner: &DigitSpinner,
    slot: SpinnerSlot,
    unit: SpinnerUnit,
) -> (String, Color, Color) {
    use crate::hud::item_ui::theme;
    match slot {
        SpinnerSlot::All => column_style(spinner, SpinnerColumn::All),
        SpinnerSlot::Cell(i) => cell_style(spinner, i),
        SpinnerSlot::Suffix => (
            format!("{} {ARROW_RIGHT}", unit.suffix()),
            theme::TEXT,
            Color::NONE,
        ),
        SpinnerSlot::Cap => (
            format!("/{}", group_digits(spinner.cap)),
            theme::MUTED,
            Color::NONE,
        ),
    }
}

fn cell_style(spinner: &DigitSpinner, cell: usize) -> (String, Color, Color) {
    use crate::hud::item_ui::theme;
    let blank = (String::new(), theme::TEXT, Color::NONE);
    let width = spinner.visible_powers().count() as u32;
    match layout().nth(cell) {
        None => blank,
        Some(CellKind::Separator { above }) if width > above => {
            (GROUP_SEPARATOR.to_string(), theme::TEXT, Color::NONE)
        }
        Some(CellKind::Separator { .. }) => blank,
        Some(CellKind::Digit(p)) if p >= width => blank,
        Some(CellKind::Digit(p)) => column_style(spinner, SpinnerColumn::Digit(p)),
    }
}

pub fn column_style(spinner: &DigitSpinner, column: SpinnerColumn) -> (String, Color, Color) {
    use crate::hud::item_ui::theme;
    match column {
        SpinnerColumn::All => (
            format!("All {ARROW_LEFT} "),
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

/// The chrome retail brackets the digits with, and the unit it writes after a
/// gil amount (.agents/skills/retail-observe/references/auction-house.md
/// "Price Set"). Drawn with the HUD font, which has both triangles.
const ARROW_LEFT: &str = "\u{25c4}";
const ARROW_RIGHT: &str = "\u{25ba}";
const GIL_UNIT: &str = " G";

// Approximate the active and edited tints in the auction-house.md recording.
const SPINNER_ACTIVE_BG: Color = Color::srgba(0.85, 0.25, 0.35, 0.85);
const SPINNER_EDITED: Color = Color::srgb(1.0, 0.62, 0.25);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hud::auction::PRICE_CAP;

    /// Walk the column cursor to the far left, wherever the cap puts the top
    /// digit.
    fn walk_left(spinner: &mut DigitSpinner) {
        for _ in 0..=PRICE_DIGITS {
            spinner.left();
        }
    }

    fn row(spinner: &DigitSpinner, unit: SpinnerUnit) -> String {
        slots()
            .map(|slot| slot_style(spinner, slot, unit).0)
            .collect()
    }

    /// "All" is a column, not a caption: it is lit only while it is the
    /// selected one, so the row never claims the whole amount over a partial
    /// value.
    #[test]
    fn the_all_column_is_only_highlighted_when_it_is_selected() {
        use crate::hud::item_ui::theme;
        let mut spinner = DigitSpinner::item(12);
        assert_eq!(
            slot_style(&spinner, SpinnerSlot::All, SpinnerUnit::Count).1,
            theme::TEXT
        );
        walk_left(&mut spinner);
        assert_eq!(spinner.column, SpinnerColumn::All);
        assert_eq!(
            slot_style(&spinner, SpinnerSlot::All, SpinnerUnit::Count).1,
            theme::CURSOR
        );
    }

    /// The retail stack-quantity picker, spelled out
    /// (.agents/skills/retail-observe/references/2026-07-17-moghouse-menu.md
    /// "Send flow" step 4 shows `All < 1 / 2 >` for a stack of two).
    #[test]
    fn a_quantity_row_reads_like_the_retail_one() {
        assert_eq!(
            row(&DigitSpinner::item(2), SpinnerUnit::Count),
            "All \u{25c4} 1/2 \u{25ba}"
        );
    }

    #[test]
    fn a_gil_row_groups_its_digits_and_carries_the_unit() {
        assert_eq!(
            row(&DigitSpinner::with_value(17_488, 9_007), SpinnerUnit::Gil),
            "All \u{25c4} 9,007/17,488 G \u{25ba}"
        );
    }

    /// Navigation moves the column and nothing else, including at the ends of
    /// the walk where there is no further column to move to.
    #[test]
    fn walking_off_either_end_never_moves_the_value() {
        let mut spinner = DigitSpinner::item(12);
        spinner.up();
        assert_eq!(spinner.value, 2);

        for _ in 0..PRICE_DIGITS + 2 {
            spinner.left();
        }
        assert_eq!(spinner.column, SpinnerColumn::All);
        assert_eq!(spinner.value, 2, "the left walk ends on All, untouched");

        for _ in 0..PRICE_DIGITS + 2 {
            spinner.right();
        }
        assert_eq!(spinner.column, SpinnerColumn::Digit(0));
        assert_eq!(spinner.value, 2, "the right walk ends on ones, untouched");
    }

    /// Every instance offers the All column, so "take the whole amount" is one
    /// reachable control rather than a shortcut key some screens bind.
    #[test]
    fn all_is_reachable_on_a_quantity_and_on_a_price() {
        for mut spinner in [DigitSpinner::item(12), DigitSpinner::new(1_180)] {
            let cap = spinner.cap;
            walk_left(&mut spinner);
            assert_eq!(spinner.column, SpinnerColumn::All);
            spinner.up();
            assert_eq!(spinner.value, cap);
            spinner.down();
            assert_eq!(spinner.value, spinner.min);
        }
    }

    #[test]
    fn quantity_bounds_apply_to_every_digit() {
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

    #[test]
    fn opens_on_ones_at_zero() {
        let s = DigitSpinner::new(PRICE_CAP);
        assert_eq!(s.value, 0);
        assert_eq!(s.column, SpinnerColumn::Digit(0));
    }

    #[test]
    fn digit_steps_add_place_value_and_clamp() {
        let mut s = DigitSpinner::new(PRICE_CAP);
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
        let mut s = DigitSpinner::with_value(PRICE_CAP, 600);
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
        let mut s = DigitSpinner::new(PRICE_CAP);
        assert_eq!(s.visible_powers().collect::<Vec<_>>(), vec![0]);
        s.left();
        s.left();
        assert_eq!(s.visible_powers().collect::<Vec<_>>(), vec![2, 1, 0]);
        s.up();
        assert_eq!(s.value, 100);
        assert_eq!(s.digit_at(2), 1);
    }

    /// The group separators sit where [`ffxi_vocab::gil::group_digits`] puts
    /// them, so a spelled-out row and a formatted amount cannot disagree.
    #[test]
    fn the_cells_group_exactly_as_the_shared_formatter_does() {
        for value in [0, 999, 1_000, 24_999, 1_389_292, PRICE_CAP] {
            let spinner = DigitSpinner::with_value(PRICE_CAP, value);
            let drawn: String = cells()
                .map(|slot| slot_style(&spinner, slot, SpinnerUnit::Count).0)
                .collect();
            assert_eq!(drawn, group_digits(value), "value {value}");
        }
    }
}
