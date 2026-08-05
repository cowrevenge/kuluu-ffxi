use super::*;

/// GP_CLI_COMMAND_AUC_COMMAND, shared by c2s 0x04E and s2c 0x04C
/// (vendor/server/src/map/packets/c2s/0x04e_auc.h).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AuctionCommand {
    /// s2c only: the AH counter menu opens
    /// (vendor/server/src/map/lua/lua_baseentity.cpp CLuaBaseEntity::sendMenu).
    Open = 0x02,
    AskCommit = 0x04,
    Info = 0x05,
    WorkCheck = 0x0A,
    LotIn = 0x0B,
    /// Also the command byte on the sales-status rows pushed after
    /// WorkCheck/Info/LotIn (vendor/server/src/map/utils/auctionutils.cpp
    /// RetrieveListOfItemsSoldByPlayer / ProofOfPurchase).
    LotCancel = 0x0C,
    LotCheck = 0x0D,
    Bid = 0x0E,
}

impl AuctionCommand {
    const ALL: [Self; 8] = [
        Self::Open,
        Self::AskCommit,
        Self::Info,
        Self::WorkCheck,
        Self::LotIn,
        Self::LotCancel,
        Self::LotCheck,
        Self::Bid,
    ];

    pub fn from_u8(raw: u8) -> Result<Self, DecodeError> {
        Self::ALL
            .into_iter()
            .find(|&c| c as u8 == raw)
            .ok_or(DecodeError::UnknownDiscriminant(raw))
    }
}

/// GP_AUC_PARAM ItemStacks: 1 = single item, 0 = a full stack
/// (vendor/server/src/map/packets/s2c/0x04c_auc.cpp).
pub const AUCTION_STACKS_SINGLE: u32 = 1;
pub const AUCTION_STACKS_STACK: u32 = 0;

/// Per-player sale slots 0..=6 (AucWorkIndex range in
/// GP_CLI_COMMAND_AUC::validate, vendor/server/src/map/packets/c2s/0x04e_auc.cpp).
pub const AUCTION_SLOT_COUNT: u8 = 7;

/// Slot-less commands carry AucWorkIndex -1; WorkCheck requires it
/// (GP_CLI_COMMAND_AUC::validate).
pub const AUCTION_WORK_INDEX_NONE: i8 = -1;

/// Validator cap on Commission/LimitPrice/BidPrice
/// (GP_CLI_COMMAND_AUC::validate).
pub const AUCTION_PRICE_MAX: u32 = 999_999_999;

/// s2c Result on menu-open and other success pushes
/// (GP_SERV_COMMAND_AUC constructors, vendor/server/src/map/packets/s2c/0x04c_auc.cpp).
pub const AUCTION_RESULT_OPEN: u8 = 1;

/// s2c ResultStatus marking an AskCommit fee quote
/// (GP_SERV_COMMAND_AUC fee-quote constructor).
pub const AUCTION_RESULT_STATUS_FEE: u8 = 0x02;

/// Parcel.Stat on a populated sales-status row
/// (GP_SERV_COMMAND_AUC slot constructor).
pub const AUCTION_SALE_STAT_LISTED: u8 = 0x03;

/// GP_AUC_PARAM_ASKCOMMIT (vendor/server/src/map/packets/c2s/0x04e_auc.h).
/// s2c: the listing-fee quote; a rejected item comes back zeroed with the
/// message code in `Auction::result`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuctionAskCommit {
    pub commission: u32,
    pub item_work_index: u16,
    pub item_no: u16,
    pub item_stacks: u32,
}

/// GP_AUC_PARAM_BID (vendor/server/src/map/packets/c2s/0x04e_auc.h).
/// s2c: the bid echo with `Auction::result` as the outcome message code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuctionBid {
    pub bid_price: u32,
    pub item_no: u16,
    pub item_stacks: u32,
}

/// GP_AUC_PARAM_LOT (vendor/server/src/map/packets/c2s/0x04e_auc.h).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuctionLotIn {
    pub limit_price: u32,
    pub item_work_index: u16,
    pub item_stacks: u32,
}

/// GP_AUC_PARAM union member keyed by the command byte
/// (vendor/server/src/map/packets/c2s/0x04e_auc.h).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuctionParam {
    AskCommit(AuctionAskCommit),
    Bid(AuctionBid),
    LotIn(AuctionLotIn),
    None,
}

/// GP_AUC_BOX (vendor/server/src/map/packets/c2s/0x04e_auc.h), populated on
/// sales-status rows (GP_SERV_COMMAND_AUC slot constructors); `name` is the
/// seller on the LotCancel keep-item push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuctionSaleSlot {
    pub stat: u8,
    pub item_index: u8,
    pub name: String,
    pub item_no: u16,
    pub quantity: u8,
    pub category: u8,
    pub price: u32,
    pub market_no: u32,
    pub lot_no: u32,
    pub timestamp: u32,
}

impl AuctionSaleSlot {
    const STAT_OFFSET: usize = 0;
    const ITEM_INDEX_OFFSET: usize = 2;
    const NAME_OFFSET: usize = 4;
    const ITEM_NO_OFFSET: usize = 20;
    const QUANTITY_OFFSET: usize = 22;
    const CATEGORY_OFFSET: usize = 23;
    const PRICE_OFFSET: usize = 24;
    const MARKET_NO_OFFSET: usize = 28;
    const LOT_NO_OFFSET: usize = 32;
    const TIMESTAMP_OFFSET: usize = 36;
    pub const SIZE: usize = Self::TIMESTAMP_OFFSET + 4;

    fn decode(body: &[u8]) -> Self {
        let rd32 = |o: usize| u32::from_le_bytes(body[o..o + 4].try_into().unwrap());
        let rd16 = |o: usize| u16::from_le_bytes([body[o], body[o + 1]]);
        Self {
            stat: body[Self::STAT_OFFSET],
            item_index: body[Self::ITEM_INDEX_OFFSET],
            name: name_at(body, Self::NAME_OFFSET),
            item_no: rd16(Self::ITEM_NO_OFFSET),
            quantity: body[Self::QUANTITY_OFFSET],
            category: body[Self::CATEGORY_OFFSET],
            price: rd32(Self::PRICE_OFFSET),
            market_no: rd32(Self::MARKET_NO_OFFSET),
            lot_no: rd32(Self::LOT_NO_OFFSET),
            timestamp: rd32(Self::TIMESTAMP_OFFSET),
        }
    }
}

/// GP_SERV_COMMAND_AUC body (vendor/server/src/map/packets/s2c/0x04c_auc.h
/// PacketData, same layout as the c2s struct minus the 4-byte subpacket
/// header). `result` carries LSB's u8 message codes (0xE5 "no space", 0xC5
/// failed bid, 197, 246…), so it is decoded unsigned. `sale` is present when
/// Parcel.Stat is nonzero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Auction {
    pub command: AuctionCommand,
    pub work_index: i8,
    pub result: u8,
    pub result_status: u8,
    pub param: AuctionParam,
    pub sale: Option<AuctionSaleSlot>,
}

impl Auction {
    const COMMAND_OFFSET: usize = 0;
    const WORK_INDEX_OFFSET: usize = 1;
    const RESULT_OFFSET: usize = 2;
    const RESULT_STATUS_OFFSET: usize = 3;
    const PARAM_OFFSET: usize = 4;
    // GP_AUC_PARAM is a 12-byte union — its largest members
    // (GP_AUC_PARAM_LOT/BID/ASKCOMMIT/TRANS) are each 12 bytes.
    const PARCEL_OFFSET: usize = Self::PARAM_OFFSET + 12;
    pub const SIZE: usize = Self::PARCEL_OFFSET + AuctionSaleSlot::SIZE;

    const PARAM_PRICE_OFFSET: usize = Self::PARAM_OFFSET;
    const PARAM_WORD0_OFFSET: usize = Self::PARAM_OFFSET + 4;
    const PARAM_WORD1_OFFSET: usize = Self::PARAM_OFFSET + 6;
    const PARAM_STACKS_OFFSET: usize = Self::PARAM_OFFSET + 8;

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::SIZE {
            return Err(DecodeError::Truncated(Self::SIZE, body.len()));
        }
        let rd32 = |o: usize| u32::from_le_bytes(body[o..o + 4].try_into().unwrap());
        let rd16 = |o: usize| u16::from_le_bytes([body[o], body[o + 1]]);
        let command = AuctionCommand::from_u8(body[Self::COMMAND_OFFSET])?;
        let param = match command {
            AuctionCommand::AskCommit => AuctionParam::AskCommit(AuctionAskCommit {
                commission: rd32(Self::PARAM_PRICE_OFFSET),
                item_work_index: rd16(Self::PARAM_WORD0_OFFSET),
                item_no: rd16(Self::PARAM_WORD1_OFFSET),
                item_stacks: rd32(Self::PARAM_STACKS_OFFSET),
            }),
            AuctionCommand::Bid => AuctionParam::Bid(AuctionBid {
                bid_price: rd32(Self::PARAM_PRICE_OFFSET),
                item_no: rd16(Self::PARAM_WORD0_OFFSET),
                item_stacks: rd32(Self::PARAM_STACKS_OFFSET),
            }),
            AuctionCommand::LotIn => AuctionParam::LotIn(AuctionLotIn {
                limit_price: rd32(Self::PARAM_PRICE_OFFSET),
                item_work_index: rd16(Self::PARAM_WORD0_OFFSET),
                item_stacks: rd32(Self::PARAM_STACKS_OFFSET),
            }),
            _ => AuctionParam::None,
        };
        let parcel = &body[Self::PARCEL_OFFSET..Self::PARCEL_OFFSET + AuctionSaleSlot::SIZE];
        let sale =
            (parcel[AuctionSaleSlot::STAT_OFFSET] != 0).then(|| AuctionSaleSlot::decode(parcel));
        Ok(Self {
            command,
            work_index: body[Self::WORK_INDEX_OFFSET] as i8,
            result: body[Self::RESULT_OFFSET],
            result_status: body[Self::RESULT_STATUS_OFFSET],
            param,
            sale,
        })
    }
}

// sizeof(GP_SERV_COMMAND_AUC::PacketData) — a widened GP_AUC_PARAM or
// GP_AUC_BOX upstream must be caught here, not at runtime.
const _: () = assert!(Auction::SIZE == 56 && AuctionSaleSlot::SIZE == 40);

const NAME_LEN: usize = 16;

fn name_at(body: &[u8], offset: usize) -> String {
    let bytes = &body[offset..offset + NAME_LEN];
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Menu-open push: the action-only GP_SERV_COMMAND_AUC constructor —
    /// Command=Open, AucWorkIndex=-1, Result=1, everything else zero.
    #[test]
    fn open_reads_work_index_and_result() {
        let mut buf = vec![0u8; Auction::SIZE];
        buf[0] = AuctionCommand::Open as u8;
        buf[1] = -1i8 as u8;
        buf[2] = AUCTION_RESULT_OPEN;
        let a = Auction::decode(&buf).expect("decode");
        assert_eq!(a.command, AuctionCommand::Open);
        assert_eq!(a.work_index, AUCTION_WORK_INDEX_NONE);
        assert_eq!(a.result, AUCTION_RESULT_OPEN);
        assert_eq!(a.result_status, 0);
        assert_eq!(a.param, AuctionParam::None);
        assert!(a.sale.is_none());
    }

    /// Fee-quote push: the CItem GP_SERV_COMMAND_AUC constructor —
    /// ResultStatus=0x02, Param.AskCommit carries fee/slot/item/stacks,
    /// Parcel.MarketNo=4 with Stat still zero (no sale row).
    #[test]
    fn ask_commit_quote_reads_lsb_offsets() {
        let mut buf = vec![0u8; Auction::SIZE];
        buf[0] = AuctionCommand::AskCommit as u8;
        buf[1] = -1i8 as u8;
        buf[2] = AUCTION_RESULT_OPEN;
        buf[3] = AUCTION_RESULT_STATUS_FEE;
        buf[4..8].copy_from_slice(&9u32.to_le_bytes()); // Commission
        buf[8..10].copy_from_slice(&5u16.to_le_bytes()); // ItemWorkIndex
        buf[10..12].copy_from_slice(&4570u16.to_le_bytes()); // ItemNo (Bird Egg)
        buf[12..16].copy_from_slice(&AUCTION_STACKS_STACK.to_le_bytes());
        buf[44..48].copy_from_slice(&4u32.to_le_bytes()); // Parcel.MarketNo
        let a = Auction::decode(&buf).expect("decode");
        assert_eq!(a.result_status, AUCTION_RESULT_STATUS_FEE);
        assert_eq!(
            a.param,
            AuctionParam::AskCommit(AuctionAskCommit {
                commission: 9,
                item_work_index: 5,
                item_no: 4570,
                item_stacks: AUCTION_STACKS_STACK,
            })
        );
        assert!(a.sale.is_none(), "MarketNo alone is not a sale row");
    }

    /// Sales-status row: the slot GP_SERV_COMMAND_AUC constructors (plain and
    /// keepItem) — Parcel Stat=0x03, ItemIndex=1, Name, ItemNo,
    /// ItemQuantity=1-stack, ItemCategory=2, Price, MarketNo=4.
    #[test]
    fn sales_status_row_reads_parcel() {
        let mut buf = vec![0u8; Auction::SIZE];
        buf[0] = AuctionCommand::LotCheck as u8;
        buf[1] = 3; // AucWorkIndex = slot
        buf[2] = AUCTION_RESULT_OPEN;
        buf[16] = AUCTION_SALE_STAT_LISTED;
        buf[18] = 0x01; // ItemIndex
        buf[20..24].copy_from_slice(b"Atti"); // Name, NUL-padded
        buf[36..38].copy_from_slice(&4570u16.to_le_bytes()); // ItemNo
        buf[38] = 1; // ItemQuantity (1 - stack)
        buf[39] = 0x02; // ItemCategory
        buf[40..44].copy_from_slice(&1180u32.to_le_bytes()); // Price
        buf[44..48].copy_from_slice(&4u32.to_le_bytes()); // MarketNo
        buf[48..52].copy_from_slice(&7u32.to_le_bytes()); // LotNo
        buf[52..56].copy_from_slice(&0x66B0_1234u32.to_le_bytes()); // TimeStamp
        let a = Auction::decode(&buf).expect("decode");
        assert_eq!(a.command, AuctionCommand::LotCheck);
        assert_eq!(a.work_index, 3);
        assert_eq!(a.param, AuctionParam::None);
        let sale = a.sale.expect("Stat != 0 populates the sale row");
        assert_eq!(sale.stat, AUCTION_SALE_STAT_LISTED);
        assert_eq!(sale.item_index, 0x01);
        assert_eq!(sale.name, "Atti");
        assert_eq!(sale.item_no, 4570);
        assert_eq!(sale.quantity, 1);
        assert_eq!(sale.category, 0x02);
        assert_eq!(sale.price, 1180);
        assert_eq!(sale.market_no, 4);
        assert_eq!(sale.lot_no, 7);
        assert_eq!(sale.timestamp, 0x66B0_1234);
    }

    /// Failed bid: the message GP_SERV_COMMAND_AUC constructor — Result=0xC5
    /// with the echoed Param.Bid (ItemStacks = stack size for a stack bid).
    #[test]
    fn failed_bid_reads_message_and_echo() {
        const BID_FAILED: u8 = 0xC5; // auctionutils.cpp PurchasingItems
        let mut buf = vec![0u8; Auction::SIZE];
        buf[0] = AuctionCommand::Bid as u8;
        buf[2] = BID_FAILED;
        buf[4..8].copy_from_slice(&490u32.to_le_bytes()); // BidPrice
        buf[8..10].copy_from_slice(&17440u16.to_le_bytes()); // ItemNo
        buf[12..16].copy_from_slice(&12u32.to_le_bytes()); // ItemStacks = stack size
        let a = Auction::decode(&buf).expect("decode");
        assert_eq!(a.result, BID_FAILED);
        assert_eq!(
            a.param,
            AuctionParam::Bid(AuctionBid {
                bid_price: 490,
                item_no: 17440,
                item_stacks: 12,
            })
        );
    }

    #[test]
    fn command_roundtrips_and_rejects_unknown() {
        for c in AuctionCommand::ALL {
            assert_eq!(AuctionCommand::from_u8(c as u8).unwrap(), c);
        }
        assert!(matches!(
            AuctionCommand::from_u8(0x03),
            Err(DecodeError::UnknownDiscriminant(0x03))
        ));
    }

    #[test]
    fn truncated_body_errors() {
        assert!(matches!(
            Auction::decode(&[0u8; Auction::SIZE - 1]),
            Err(DecodeError::Truncated(n, _)) if n == Auction::SIZE
        ));
    }
}
