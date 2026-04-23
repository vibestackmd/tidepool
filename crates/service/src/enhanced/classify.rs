//! Map a tx's invoked program IDs → Helius-style `(type, source)`.
//!
//! Coverage is deliberately narrow: only the programs where we can
//! produce a useful classification without running full per-program
//! parsers. Everything else collapses to `UNKNOWN`. Callers never
//! get a "confidently wrong" classification.

use crate::cnft::parser::{
    BURN_DISC, BURN_V2_DISC, MINT_TO_COLLECTION_V1_DISC, MINT_V1_DISC, MINT_V2_DISC, TRANSFER_DISC,
    TRANSFER_V2_DISC,
};

pub const SYSTEM_PROGRAM_ID: &str = "11111111111111111111111111111111";
pub const SPL_TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
pub const TOKEN_2022_PROGRAM_ID: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
pub const BUBBLEGUM_PROGRAM_ID: &str = "BGUMAp9Gq7iTEuizy4pqaxsTyUCBK68MDfK752saRPUY";
pub const MPL_TOKEN_METADATA_PROGRAM: &str = "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s";
pub const MPL_CORE_PROGRAM: &str = "CoREENxT6tW1HoK8ypY1SxRMZTcVPm7R94rH4PZNhX7d";
/// Jupiter v6 aggregator — the dominant Solana swap router.
pub const JUPITER_V6_PROGRAM_ID: &str = "JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4";
/// pump.fun bonding-curve mint + trade program.
pub const PUMP_FUN_PROGRAM_ID: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";
/// Metaplex Candy Machine Core (v3 line).
pub const CANDY_MACHINE_PROGRAM_ID: &str = "CMACYFENjoBMHzapRXyo1JZkVS6EtaDDzkjMrmQLvr4J";

/// Result of classifying one tx. Kept as a struct rather than a
/// tuple so call-site shape is self-documenting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnhancedClass {
    pub tx_type: &'static str,
    pub source: &'static str,
    pub description: String,
}

/// Walk the outer instructions and decide (type, source).
///
/// `instructions` is the compact-ix array with base58 `data` strings
/// and `program_id` resolved to base58 strings.
#[must_use]
#[allow(clippy::too_many_lines)] // priority-order lookup table; splitting hurts readability
pub fn classify(instructions: &[InstructionView<'_>]) -> EnhancedClass {
    // Priority order: more specific classifications win.

    // Swap-family routers first — Jupiter invocations usually stack a
    // Metaplex or SPL-Token ix under them, so if we checked NFT/token
    // paths first we'd mis-label a swap as a mint.
    for ix in instructions {
        if ix.program_id == JUPITER_V6_PROGRAM_ID {
            return EnhancedClass {
                tx_type: "SWAP",
                source: "JUPITER",
                description: "Jupiter swap".into(),
            };
        }
    }

    // pump.fun + Candy Machine Core — dedicated mint sources with
    // their own enhanced labels. Keep before the generic Metaplex
    // check so the specific source wins.
    for ix in instructions {
        match ix.program_id {
            PUMP_FUN_PROGRAM_ID => {
                return EnhancedClass {
                    tx_type: "NFT_MINT",
                    source: "PUMP_FUN",
                    description: "pump.fun activity".into(),
                };
            }
            CANDY_MACHINE_PROGRAM_ID => {
                return EnhancedClass {
                    tx_type: "NFT_MINT",
                    source: "CANDY_MACHINE",
                    description: "Candy Machine mint".into(),
                };
            }
            _ => {}
        }
    }

    // Compressed-NFT ops via Bubblegum — check discriminators to pick
    // the specific variant. Covers both V1 and V2 wire formats.
    for ix in instructions {
        if ix.program_id == BUBBLEGUM_PROGRAM_ID {
            if let Some(disc) = ix.data.get(..8) {
                let disc: [u8; 8] = disc.try_into().unwrap_or_default();
                let label = match disc {
                    d if d == MINT_V1_DISC
                        || d == MINT_TO_COLLECTION_V1_DISC
                        || d == MINT_V2_DISC =>
                    {
                        Some("COMPRESSED_NFT_MINT")
                    }
                    d if d == TRANSFER_DISC || d == TRANSFER_V2_DISC => {
                        Some("COMPRESSED_NFT_TRANSFER")
                    }
                    d if d == BURN_DISC || d == BURN_V2_DISC => Some("COMPRESSED_NFT_BURN"),
                    _ => None,
                };
                if let Some(t) = label {
                    return EnhancedClass {
                        tx_type: t,
                        source: "BUBBLEGUM",
                        description: format!(
                            "Compressed NFT {}",
                            t.to_lowercase().replace('_', " ")
                        ),
                    };
                }
            }
        }
    }

    // Token Metadata or MplCore → NFT_MINT. We treat any invocation
    // of these programs as a mint-family event at this coarseness —
    // fine-grained update/verify classification needs program-specific
    // parsers we don't ship here.
    for ix in instructions {
        if ix.program_id == MPL_TOKEN_METADATA_PROGRAM {
            return EnhancedClass {
                tx_type: "NFT_MINT",
                source: "METAPLEX",
                description: "Token Metadata activity".into(),
            };
        }
        if ix.program_id == MPL_CORE_PROGRAM {
            return EnhancedClass {
                tx_type: "NFT_MINT",
                source: "MPL_CORE",
                description: "MplCore asset activity".into(),
            };
        }
    }

    // Pure SPL-Token or Token-2022 ixs (no others outside system)
    // classify as TRANSFER. Real Helius further differentiates
    // BURN / CLOSE / APPROVE, but we keep it flat.
    let all_system_or_token = instructions.iter().all(|ix| {
        matches!(
            ix.program_id,
            SYSTEM_PROGRAM_ID | SPL_TOKEN_PROGRAM_ID | TOKEN_2022_PROGRAM_ID
        )
    });
    if !instructions.is_empty() && all_system_or_token {
        let has_token = instructions.iter().any(|ix| {
            ix.program_id == SPL_TOKEN_PROGRAM_ID || ix.program_id == TOKEN_2022_PROGRAM_ID
        });
        return EnhancedClass {
            tx_type: "TRANSFER",
            source: if has_token {
                "SOLANA_TOKEN_PROGRAM"
            } else {
                "SYSTEM_PROGRAM"
            },
            description: if has_token {
                "Token transfer".into()
            } else {
                "Native SOL transfer".into()
            },
        };
    }

    EnhancedClass {
        tx_type: "UNKNOWN",
        source: "UNKNOWN",
        description: String::new(),
    }
}

/// Cheap projection of an instruction for classification. Owns the
/// decoded data because bs58-decoding happens here; classifier code
/// doesn't want to care where the bytes come from.
#[derive(Debug, Clone)]
pub struct InstructionView<'a> {
    pub program_id: &'a str,
    pub data: Vec<u8>,
}

/// Materialize `InstructionView`s from a decoded `accountKeys` pool +
/// the raw compact instructions. Skips any ix whose
/// `program_id_index` doesn't resolve inside `account_keys`.
#[must_use]
pub fn instruction_views<'a>(
    account_keys: &'a [String],
    instructions: &[RawInstruction],
) -> Vec<InstructionView<'a>> {
    instructions
        .iter()
        .filter_map(|ix| {
            let program_id = account_keys.get(ix.program_id_index as usize)?.as_str();
            let data = bs58::decode(&ix.data).into_vec().unwrap_or_default();
            Some(InstructionView { program_id, data })
        })
        .collect()
}

/// Minimal deserialize target for a compact outer ix. `data` is
/// base58 per Solana's `getTransaction` default encoding.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct RawInstruction {
    #[serde(rename = "programIdIndex")]
    pub program_id_index: u32,
    #[serde(default)]
    pub accounts: Vec<u32>,
    pub data: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(pid: &str, data: Vec<u8>) -> InstructionView<'_> {
        InstructionView {
            program_id: pid,
            data,
        }
    }

    #[test]
    fn bubblegum_mint_v1_classifies_as_compressed_mint() {
        let c = classify(&[view(BUBBLEGUM_PROGRAM_ID, MINT_V1_DISC.to_vec())]);
        assert_eq!(c.tx_type, "COMPRESSED_NFT_MINT");
        assert_eq!(c.source, "BUBBLEGUM");
    }

    #[test]
    fn bubblegum_transfer_v2_classifies_as_compressed_transfer() {
        let c = classify(&[view(BUBBLEGUM_PROGRAM_ID, TRANSFER_V2_DISC.to_vec())]);
        assert_eq!(c.tx_type, "COMPRESSED_NFT_TRANSFER");
    }

    #[test]
    fn system_program_only_is_native_transfer() {
        let c = classify(&[view(SYSTEM_PROGRAM_ID, vec![2, 0, 0, 0])]);
        assert_eq!(c.tx_type, "TRANSFER");
        assert_eq!(c.source, "SYSTEM_PROGRAM");
    }

    #[test]
    fn spl_token_only_is_token_transfer() {
        let c = classify(&[view(SPL_TOKEN_PROGRAM_ID, vec![3, 0, 0, 0])]);
        assert_eq!(c.tx_type, "TRANSFER");
        assert_eq!(c.source, "SOLANA_TOKEN_PROGRAM");
    }

    #[test]
    fn mixed_system_and_token_is_token_transfer() {
        let c = classify(&[
            view(SYSTEM_PROGRAM_ID, vec![0]),
            view(SPL_TOKEN_PROGRAM_ID, vec![0]),
        ]);
        assert_eq!(c.tx_type, "TRANSFER");
        assert_eq!(c.source, "SOLANA_TOKEN_PROGRAM");
    }

    #[test]
    fn metaplex_invocation_is_nft_mint() {
        let c = classify(&[view(MPL_TOKEN_METADATA_PROGRAM, vec![0])]);
        assert_eq!(c.tx_type, "NFT_MINT");
        assert_eq!(c.source, "METAPLEX");
    }

    #[test]
    fn unknown_program_is_unknown() {
        let c = classify(&[view(
            "SomeUnrelatedProgram1111111111111111111111111",
            vec![0],
        )]);
        assert_eq!(c.tx_type, "UNKNOWN");
        assert_eq!(c.source, "UNKNOWN");
    }

    #[test]
    fn empty_instructions_is_unknown() {
        let c = classify(&[]);
        assert_eq!(c.tx_type, "UNKNOWN");
    }
}
