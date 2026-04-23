//! NFT event breakouts. Given a classified `EnhancedTransaction`, pull
//! out a heuristic `NftEvent` for the NFT-flavored types. Deliberately
//! a narrow heuristic — we're pattern-matching on our own classifier's
//! known outputs, not running real Metaplex/Bubblegum parsers. When
//! the pattern doesn't fit cleanly we return `None`; the top-level
//! `events.nft` is then omitted.

use super::types::{
    EnhancedNativeTransfer, EnhancedTokenTransfer, EnhancedTransaction, NftEvent, NftEventMint,
};

/// Derive an NFT event from a pre-populated `EnhancedTransaction`.
/// Runs after classification + transfer extraction, so we have
/// `tx_type`, `source`, `native_transfers`, `token_transfers` all
/// filled in.
#[must_use]
pub fn derive_nft_event(tx: &EnhancedTransaction) -> Option<NftEvent> {
    // Only populate for NFT-family classifications. Everything else
    // gets no events.nft.
    if !is_nft_type(&tx.tx_type) {
        return None;
    }

    // Mint identifier(s): token transfers with amount=1 are almost
    // always NFT moves (supply-1 assets). Dedup by mint so a single
    // NFT crossing two token accounts (move + close) isn't counted twice.
    let nfts = collect_nfts(&tx.token_transfers);

    // Sale-like inference: the largest native transfer is the most
    // likely "paid" lamport line. We don't distinguish marketplace
    // royalties from principal — callers who need that precision
    // should drive real Helius.
    let (amount, buyer, seller) = infer_principal_transfer(&tx.native_transfers);

    Some(NftEvent {
        event_type: tx.tx_type.clone(),
        source: tx.source.clone(),
        nfts: if nfts.is_empty() { None } else { Some(nfts) },
        amount,
        buyer,
        seller,
    })
}

fn is_nft_type(t: &str) -> bool {
    matches!(
        t,
        "NFT_MINT"
            | "NFT_TRANSFER"
            | "NFT_BURN"
            | "NFT_SALE"
            | "COMPRESSED_NFT_MINT"
            | "COMPRESSED_NFT_TRANSFER"
            | "COMPRESSED_NFT_BURN"
    )
}

fn collect_nfts(transfers: &[EnhancedTokenTransfer]) -> Vec<NftEventMint> {
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for t in transfers {
        if t.token_amount == 1 && seen.insert(t.mint.clone()) {
            // Token standard is a rough call — supply-1 alone can't
            // distinguish NonFungible from NonFungibleEdition without
            // fetching the mint. We tag with the coarse-grained
            // "NonFungible" label that covers the 99% case; consumers
            // needing precision should fetch getAsset.
            out.push(NftEventMint {
                mint: t.mint.clone(),
                token_standard: "NonFungible".into(),
            });
        }
    }
    out
}

fn infer_principal_transfer(
    transfers: &[EnhancedNativeTransfer],
) -> (Option<u64>, Option<String>, Option<String>) {
    let biggest = transfers.iter().max_by_key(|t| t.amount);
    match biggest {
        Some(t) if t.amount > 0 => (
            Some(t.amount),
            Some(t.to_user_account.clone()),
            Some(t.from_user_account.clone()),
        ),
        _ => (None, None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enhanced::types::{EnhancedEvents, EnhancedInstruction};

    fn base(tx_type: &str, source: &str) -> EnhancedTransaction {
        EnhancedTransaction {
            signature: "SIG".into(),
            slot: 1,
            timestamp: Some(1_700_000_000),
            tx_type: tx_type.into(),
            source: source.into(),
            fee: 5000,
            fee_payer: "FEE_PAYER".into(),
            description: "test".into(),
            native_transfers: vec![],
            token_transfers: vec![],
            instructions: Vec::<EnhancedInstruction>::new(),
            account_data: Vec::new(),
            events: EnhancedEvents::default(),
            lighthouse_data: None,
            transaction_error: None,
        }
    }

    #[test]
    fn transfer_type_returns_no_nft_event() {
        let tx = base("TRANSFER", "SYSTEM_PROGRAM");
        assert!(derive_nft_event(&tx).is_none());
    }

    #[test]
    fn compressed_mint_with_noop_transfers_still_returns_event() {
        let tx = base("COMPRESSED_NFT_MINT", "BUBBLEGUM");
        let got = derive_nft_event(&tx).expect("Some");
        assert_eq!(got.event_type, "COMPRESSED_NFT_MINT");
        assert_eq!(got.source, "BUBBLEGUM");
        assert!(got.nfts.is_none());
        assert!(got.amount.is_none());
    }

    #[test]
    fn nft_transfer_extracts_single_mint_from_amount_one() {
        let mut tx = base("NFT_TRANSFER", "METAPLEX");
        tx.token_transfers = vec![EnhancedTokenTransfer {
            from_user_account: Some("SELLER".into()),
            to_user_account: Some("BUYER".into()),
            from_token_account: Some("ATA_A".into()),
            to_token_account: Some("ATA_B".into()),
            mint: "MINT_A".into(),
            token_amount: 1,
            token_standard: None,
        }];
        tx.native_transfers = vec![EnhancedNativeTransfer {
            from_user_account: "BUYER".into(),
            to_user_account: "SELLER".into(),
            amount: 1_000_000_000,
        }];
        let got = derive_nft_event(&tx).expect("Some");
        let nfts = got.nfts.expect("mints pulled");
        assert_eq!(nfts.len(), 1);
        assert_eq!(nfts[0].mint, "MINT_A");
        assert_eq!(got.amount, Some(1_000_000_000));
        assert_eq!(got.buyer.as_deref(), Some("SELLER"));
        assert_eq!(got.seller.as_deref(), Some("BUYER"));
    }

    #[test]
    fn high_amount_token_transfers_are_not_counted_as_nfts() {
        // Fungible token transfer — should NOT appear in nfts[].
        let mut tx = base("NFT_MINT", "METAPLEX");
        tx.token_transfers = vec![EnhancedTokenTransfer {
            from_user_account: Some("A".into()),
            to_user_account: Some("B".into()),
            from_token_account: Some("ATA_A".into()),
            to_token_account: Some("ATA_B".into()),
            mint: "USDC".into(),
            token_amount: 1_000_000, // fungible, not NFT
            token_standard: None,
        }];
        let got = derive_nft_event(&tx).expect("Some");
        assert!(got.nfts.is_none());
    }

    #[test]
    fn duplicate_mint_in_transfers_dedups() {
        let mut tx = base("NFT_TRANSFER", "METAPLEX");
        tx.token_transfers = vec![
            EnhancedTokenTransfer {
                from_user_account: None,
                to_user_account: None,
                from_token_account: None,
                to_token_account: None,
                mint: "MINT_A".into(),
                token_amount: 1,
                token_standard: None,
            },
            EnhancedTokenTransfer {
                from_user_account: None,
                to_user_account: None,
                from_token_account: None,
                to_token_account: None,
                mint: "MINT_A".into(),
                token_amount: 1,
                token_standard: None,
            },
        ];
        let got = derive_nft_event(&tx).expect("Some");
        assert_eq!(got.nfts.as_ref().unwrap().len(), 1);
    }
}
