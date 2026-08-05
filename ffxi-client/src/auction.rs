//! Client-side Auction House driver for the map-side sub-protocol (c2s 0x04E /
//! s2c 0x04C, vendor/server/src/map/packets/{c2s/0x04e_auc,s2c/0x04c_auc}.cpp,
//! utils/auctionutils.cpp). Mirrors [`crate::delivery_box`]: the session owns
//! the sends — this machine turns decoded [`Auction`] pushes into
//! [`AgentEvent`]s and follow-up sends, and sequences the two-phase sell
//! (AskCommit fee quote → LotIn confirm).

use ffxi_proto::decode::{
    Auction, AuctionCommand, AuctionParam, AUCTION_RESULT_OPEN, AUCTION_SLOT_COUNT,
    AUCTION_STACKS_SINGLE, AUCTION_STACKS_STACK,
};

use crate::state::{AgentEvent, AhFeeQuote, AhSaleStatus, AUCTION_SLOTS};

/// auctionutils::CancelSale success push (Result 0 with keepItem=false); every
/// other s2c success uses Result 1.
pub const AUCTION_CANCEL_OK: u8 = 0;

/// GP_CLI_COMMAND_AUC::validate ranges Bid's AucWorkIndex 0..=6 but
/// PurchasingItems never reads it, so any in-range value is equivalent.
pub const AUCTION_BID_WORK_INDEX: i8 = 0;

/// GP_AUC_PARAM ItemStacks wire form for a single-vs-stack flag.
pub fn stacks_wire(stack: bool) -> u32 {
    if stack {
        AUCTION_STACKS_STACK
    } else {
        AUCTION_STACKS_SINGLE
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingSell {
    pub inventory_slot: u8,
    pub item_no: u16,
    pub stack: bool,
    pub price: u32,
}

#[derive(Debug, Default)]
pub struct AuctionFlow {
    pending_sell: Option<PendingSell>,
    quoted: bool,
    sales_filled: [bool; AUCTION_SLOTS],
}

/// What an s2c push asks of the session: events to fan out, plus the retail
/// open handshake (the client answers Open with WorkCheck — observation record
/// .agents/skills/retail-observe/references/auction-house.md).
#[derive(Debug, Default)]
pub struct AucOutcome {
    pub events: Vec<AgentEvent>,
    pub send_work_check: bool,
}

impl AuctionFlow {
    /// Phase 1: record the sell intent whose AskCommit is about to go out. Any
    /// prior un-confirmed quote is superseded.
    pub fn request_sell(
        &mut self,
        inventory_slot: u8,
        item_no: u16,
        stack: bool,
        price: u32,
    ) -> PendingSell {
        let sell = PendingSell {
            inventory_slot,
            item_no,
            stack,
            price,
        };
        self.quoted = false;
        self.pending_sell = Some(sell);
        sell
    }

    /// Phase 2: the LotIn parameters — the recorded sell plus the target sale
    /// slot — once the AskCommit quote has landed; `None` before that.
    pub fn confirm_sell(&mut self) -> Option<(PendingSell, i8)> {
        if !self.quoted {
            return None;
        }
        let sell = self.pending_sell?;
        Some((sell, self.first_free_slot()))
    }

    // ProofOfPurchase ignores the c2s AucWorkIndex (validate only ranges it
    // 0..=6), so the first slot not known to hold a sale stands in for
    // retail's exact choice.
    fn first_free_slot(&self) -> i8 {
        self.sales_filled.iter().position(|f| !f).unwrap_or(0) as i8
    }

    pub fn on_packet(&mut self, a: &Auction) -> AucOutcome {
        let mut out = AucOutcome::default();
        match a.command {
            AuctionCommand::Open => {
                out.events.push(AgentEvent::AuctionMenuOpened);
                out.send_work_check = true;
            }
            AuctionCommand::AskCommit => {
                let quote = match (&a.param, self.pending_sell) {
                    (AuctionParam::AskCommit(q), Some(sell)) if a.result == AUCTION_RESULT_OPEN => {
                        self.quoted = true;
                        Some(AhFeeQuote {
                            fee: q.commission,
                            inventory_slot: q.item_work_index as u8,
                            item_no: q.item_no,
                            stack: q.item_stacks == AUCTION_STACKS_STACK,
                            asking_price: sell.price,
                        })
                    }
                    _ => {
                        self.pending_sell = None;
                        self.quoted = false;
                        None
                    }
                };
                out.events.push(AgentEvent::AuctionSellQuote {
                    quote,
                    result: a.result,
                });
            }
            AuctionCommand::LotIn => {
                let ok = a.result == AUCTION_RESULT_OPEN;
                if ok {
                    self.pending_sell = None;
                    self.quoted = false;
                }
                out.events.push(AgentEvent::AuctionSellResult {
                    ok,
                    result: a.result,
                });
            }
            AuctionCommand::Info => {
                if a.result == AUCTION_RESULT_OPEN {
                    self.sales_filled = Default::default();
                }
                out.events
                    .push(AgentEvent::AuctionSalesStatusReset { result: a.result });
            }
            // The 0x0C command byte carries both plain sales-status rows
            // (Result 1, from WorkCheck/Info/LotIn pushes) and the LotCancel
            // verdict (Result 0 success / error code with the row retained) —
            // see AuctionCommand::LotCancel's doc in ffxi-proto.
            AuctionCommand::LotCancel => match a.result {
                AUCTION_RESULT_OPEN => self.sales_row(a, &mut out),
                AUCTION_CANCEL_OK => {
                    if let Some(slot) = slot_of(a) {
                        self.sales_filled[slot as usize] = false;
                        out.events.push(AgentEvent::AuctionCancelResult {
                            slot,
                            ok: true,
                            result: a.result,
                        });
                        out.events
                            .push(AgentEvent::AuctionSalesSlot { slot, sale: None });
                    }
                }
                _ => {
                    if let Some(slot) = slot_of(a) {
                        out.events.push(AgentEvent::AuctionCancelResult {
                            slot,
                            ok: false,
                            result: a.result,
                        });
                        if a.sale.is_some() {
                            self.sales_row(a, &mut out);
                        }
                    }
                }
            },
            AuctionCommand::LotCheck => self.sales_row(a, &mut out),
            AuctionCommand::Bid => {
                let (price, item_no, quantity) = match a.param {
                    AuctionParam::Bid(b) => (b.bid_price, b.item_no, b.item_stacks),
                    _ => (0, 0, 0),
                };
                out.events.push(AgentEvent::AuctionBidResult {
                    ok: a.result == AUCTION_RESULT_OPEN,
                    item_no,
                    price,
                    quantity,
                    result: a.result,
                });
            }
            AuctionCommand::WorkCheck => {}
        }
        out
    }

    fn sales_row(&mut self, a: &Auction, out: &mut AucOutcome) {
        let Some(slot) = slot_of(a) else {
            return;
        };
        let sale = a.sale.as_ref().map(AhSaleStatus::from);
        self.sales_filled[slot as usize] = sale.is_some();
        out.events.push(AgentEvent::AuctionSalesSlot { slot, sale });
    }
}

fn slot_of(a: &Auction) -> Option<u8> {
    u8::try_from(a.work_index)
        .ok()
        .filter(|&s| s < AUCTION_SLOT_COUNT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ffxi_proto::decode::{
        AuctionAskCommit, AuctionBid, AuctionSaleSlot, AUCTION_RESULT_STATUS_FEE,
        AUCTION_SALE_STAT_LISTED, AUCTION_WORK_INDEX_NONE,
    };

    fn push(command: AuctionCommand, work_index: i8, result: u8) -> Auction {
        Auction {
            command,
            work_index,
            result,
            result_status: 0,
            param: AuctionParam::None,
            sale: None,
        }
    }

    fn sale_slot(item_no: u16, quantity: u8, price: u32) -> AuctionSaleSlot {
        AuctionSaleSlot {
            stat: AUCTION_SALE_STAT_LISTED,
            item_index: 1,
            name: "Atti".into(),
            item_no,
            quantity,
            category: 2,
            price,
            market_no: 4,
            lot_no: 0,
            timestamp: 0,
        }
    }

    /// auctionutils SellingItems rejection code (partially used / bad item).
    const SELL_REJECTED: u8 = 197;
    /// auctionutils PurchasingItems failed-bid code.
    const BID_FAILED: u8 = 0xC5;
    /// auctionutils CancelSale inventory-full code.
    const CANCEL_FAILED: u8 = 0xE5;

    #[test]
    fn open_emits_menu_event_and_work_check_handshake() {
        let mut f = AuctionFlow::default();
        let out = f.on_packet(&push(
            AuctionCommand::Open,
            AUCTION_WORK_INDEX_NONE,
            AUCTION_RESULT_OPEN,
        ));
        assert!(out.send_work_check);
        assert!(matches!(out.events[..], [AgentEvent::AuctionMenuOpened]));
    }

    /// The full retail sell: AskCommit quote → confirm → LotIn ok → the 0x0C
    /// sales row the server pushes for the new listing.
    #[test]
    fn sell_two_phase_quote_confirm_lot_in_then_sales_row() {
        let mut f = AuctionFlow::default();
        assert!(f.confirm_sell().is_none(), "nothing pending");

        let sell = f.request_sell(5, 4570, true, 1180);
        assert!(f.confirm_sell().is_none(), "no quote yet");

        let mut quote = push(
            AuctionCommand::AskCommit,
            AUCTION_WORK_INDEX_NONE,
            AUCTION_RESULT_OPEN,
        );
        quote.result_status = AUCTION_RESULT_STATUS_FEE;
        quote.param = AuctionParam::AskCommit(AuctionAskCommit {
            commission: 9,
            item_work_index: 5,
            item_no: 4570,
            item_stacks: AUCTION_STACKS_STACK,
        });
        let out = f.on_packet(&quote);
        match &out.events[..] {
            [AgentEvent::AuctionSellQuote {
                quote: Some(q),
                result,
            }] => {
                assert_eq!(q.fee, 9);
                assert_eq!(q.inventory_slot, 5);
                assert_eq!(q.item_no, 4570);
                assert!(q.stack);
                assert_eq!(q.asking_price, sell.price);
                assert_eq!(*result, AUCTION_RESULT_OPEN);
            }
            other => panic!("unexpected events {other:?}"),
        }

        let (confirmed, work_index) = f.confirm_sell().expect("quoted");
        assert_eq!(confirmed, sell);
        assert_eq!(work_index, 0, "no known sales yet");

        let out = f.on_packet(&push(
            AuctionCommand::LotIn,
            AUCTION_WORK_INDEX_NONE,
            AUCTION_RESULT_OPEN,
        ));
        assert!(matches!(
            out.events[..],
            [AgentEvent::AuctionSellResult { ok: true, .. }]
        ));
        assert!(f.confirm_sell().is_none(), "LotIn ok consumes the intent");

        let mut row = push(AuctionCommand::LotCancel, 0, AUCTION_RESULT_OPEN);
        row.sale = Some(sale_slot(4570, 12, 1180));
        let out = f.on_packet(&row);
        match &out.events[..] {
            [AgentEvent::AuctionSalesSlot {
                slot: 0,
                sale: Some(s),
            }] => {
                assert_eq!(s.item_no, 4570);
                assert_eq!(s.price, 1180);
            }
            other => panic!("unexpected events {other:?}"),
        }
        assert_eq!(f.first_free_slot(), 1, "slot 0 now filled");
    }

    #[test]
    fn ask_commit_rejection_clears_pending_sell() {
        let mut f = AuctionFlow::default();
        f.request_sell(3, 4509, false, 100);
        let out = f.on_packet(&push(
            AuctionCommand::AskCommit,
            AUCTION_WORK_INDEX_NONE,
            SELL_REJECTED,
        ));
        assert!(matches!(
            out.events[..],
            [AgentEvent::AuctionSellQuote {
                quote: None,
                result: SELL_REJECTED,
            }]
        ));
        assert!(f.confirm_sell().is_none());
    }

    #[test]
    fn failed_bid_reports_echoed_item_and_price() {
        let mut f = AuctionFlow::default();
        let mut bid = push(AuctionCommand::Bid, AUCTION_WORK_INDEX_NONE, BID_FAILED);
        bid.param = AuctionParam::Bid(AuctionBid {
            bid_price: 490,
            item_no: 17440,
            item_stacks: 1,
        });
        let out = f.on_packet(&bid);
        assert!(matches!(
            out.events[..],
            [AgentEvent::AuctionBidResult {
                ok: false,
                item_no: 17440,
                price: 490,
                quantity: 1,
                result: BID_FAILED,
            }]
        ));
    }

    #[test]
    fn cancel_success_empties_slot_and_failure_keeps_row() {
        let mut f = AuctionFlow::default();
        let mut row = push(AuctionCommand::LotCheck, 2, AUCTION_RESULT_OPEN);
        row.sale = Some(sale_slot(4570, 1, 500));
        f.on_packet(&row);
        assert_eq!(f.first_free_slot(), 0);

        let out = f.on_packet(&push(AuctionCommand::LotCancel, 2, AUCTION_CANCEL_OK));
        assert!(matches!(
            out.events[..],
            [
                AgentEvent::AuctionCancelResult {
                    slot: 2,
                    ok: true,
                    ..
                },
                AgentEvent::AuctionSalesSlot {
                    slot: 2,
                    sale: None,
                },
            ]
        ));

        let mut failed = push(AuctionCommand::LotCancel, 2, CANCEL_FAILED);
        failed.sale = Some(sale_slot(4570, 1, 500));
        let out = f.on_packet(&failed);
        assert!(matches!(
            out.events[..],
            [
                AgentEvent::AuctionCancelResult {
                    slot: 2,
                    ok: false,
                    result: CANCEL_FAILED,
                },
                AgentEvent::AuctionSalesSlot {
                    slot: 2,
                    sale: Some(_),
                },
            ]
        ));
    }

    #[test]
    fn info_ok_resets_tracked_slots_and_throttle_does_not() {
        let mut f = AuctionFlow::default();
        let mut row = push(AuctionCommand::LotCheck, 0, AUCTION_RESULT_OPEN);
        row.sale = Some(sale_slot(4570, 1, 500));
        f.on_packet(&row);
        assert_eq!(f.first_free_slot(), 1);

        /// auctionutils OpenListOfSales throttle code ("try again in a little
        /// while").
        const INFO_THROTTLED: u8 = 246;
        let out = f.on_packet(&push(
            AuctionCommand::Info,
            AUCTION_WORK_INDEX_NONE,
            INFO_THROTTLED,
        ));
        assert!(matches!(
            out.events[..],
            [AgentEvent::AuctionSalesStatusReset {
                result: INFO_THROTTLED
            }]
        ));
        assert_eq!(f.first_free_slot(), 1, "throttled Info keeps slots");

        f.on_packet(&push(
            AuctionCommand::Info,
            AUCTION_WORK_INDEX_NONE,
            AUCTION_RESULT_OPEN,
        ));
        assert_eq!(f.first_free_slot(), 0, "ok Info restarts the stream");
    }

    #[test]
    fn stacks_wire_matches_lsb_item_stacks_encoding() {
        assert_eq!(stacks_wire(false), AUCTION_STACKS_SINGLE);
        assert_eq!(stacks_wire(true), AUCTION_STACKS_STACK);
    }
}
