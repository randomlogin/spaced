#![cfg_attr(all(not(feature = "std"), not(test)), no_std)]

extern crate alloc;
extern crate core;

pub extern crate bitcoin;

use alloc::{vec, vec::Vec};

#[cfg(feature = "bincode")]
use bincode::{Decode, Encode};
use bitcoin::{
    psbt,
    secp256k1::{schnorr, Message},
    sighash::{Prevouts, SighashCache, TapSighashType},
    taproot,
    transaction::Version,
    Amount, OutPoint, ScriptBuf, Transaction, TxIn, TxOut, Txid, Witness,
};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::{
    constants::{BID_PSBT_INPUT_SEQUENCE, BID_PSBT_TX_LOCK_TIME, BID_PSBT_TX_VERSION},
    slabel::SLabel,
};

pub mod constants;
pub mod errors;
pub mod hasher;
pub mod prepare;
pub mod script;
pub mod slabel;
pub mod validate;

#[derive(Clone, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "bincode", derive(Encode, Decode))]
pub struct FullSpaceOut {
    #[cfg_attr(feature = "bincode", bincode(with_serde))]
    pub txid: Txid,

    #[cfg_attr(feature = "serde", serde(flatten))]
    pub spaceout: SpaceOut,
}

/// Spaces transaction output
/// This structure is a superset of [bitcoin::TxOut]
#[derive(Clone, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "bincode", derive(Encode, Decode))]
pub struct SpaceOut {
    pub n: usize,
    /// Any space associated with this output
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub space: Option<Space>,
    /// The value of the output, in satoshis.
    #[cfg_attr(feature = "bincode", bincode(with_serde))]
    pub value: Amount,
    /// The script which must be satisfied for the output to be spent.
    #[cfg_attr(feature = "bincode", bincode(with_serde))]
    pub script_pubkey: ScriptBuf,
}

#[derive(Clone, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "bincode", derive(Encode, Decode))]
pub struct Space {
    /// The target is the Space name if a spend does not follow
    /// protocol rules the target space will be disassociated from future
    /// spends
    pub name: SLabel,
    // Space specific spending conditions
    pub covenant: Covenant,
}

#[derive(Clone, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "bincode", derive(Encode, Decode))]
#[cfg_attr(feature = "serde", serde(tag = "type"))]
pub enum Covenant {
    #[cfg_attr(feature = "serde", serde(rename = "bid"))]
    Bid {
        /// The current burn increment
        #[cfg_attr(feature = "bincode", bincode(with_serde))]
        burn_increment: Amount,
        /// The signature of the bid psbt
        #[cfg_attr(feature = "bincode", bincode(with_serde))]
        signature: schnorr::Signature,
        /// Total amount of BTC burned during auction lifetime
        /// including the current burn increment
        #[cfg_attr(feature = "bincode", bincode(with_serde))]
        total_burned: Amount,
        /// Block height at which he space may be safely registered
        /// by winning bidder.
        /// `None` if in pre-auctions.
        claim_height: Option<u32>,
    },
    #[cfg_attr(feature = "serde", serde(rename = "transfer"))]
    /// Space may be transferred by its current owner
    Transfer {
        /// Block height at which this covenant expires
        expire_height: u32,
        // Any data associated with this Space
        data: Option<Bytes>,
    },
    /// Using a reserved op code during a spend
    /// Space will be locked until a future upgrade
    #[cfg_attr(feature = "serde", serde(rename = "reserved"))]
    Reserved,
}

#[derive(Copy, Clone, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "bincode", derive(Encode, Decode))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case", tag = "reason"))]
pub enum RevokeReason {
    /// Space was prematurely spent during the auctions phase
    PrematureClaim,
    /// Space output was spent either by spending it directly
    /// Space was transferred without following Input N => Output N+1 rule
    BadSpend,
    Expired,
    #[cfg_attr(feature = "serde", serde(untagged))]
    BidPsbt(BidPsbtReason),
}

#[derive(Copy, Clone, PartialEq, Debug, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case", tag = "reason")
)]
#[cfg_attr(feature = "bincode", derive(Encode, Decode))]
pub enum RejectReason {
    AlreadyExists,
    #[cfg_attr(feature = "serde", serde(untagged))]
    BidPsbt(BidPsbtReason),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(tag = "reason")
)]
#[cfg_attr(feature = "bincode", derive(Encode, Decode))]
pub enum BidPsbtReason {
    #[cfg_attr(feature = "serde", serde(rename = "bid_psbt_required"))]
    Required,
    #[cfg_attr(feature = "serde", serde(rename = "bid_psbt_low_bid_amount"))]
    LowBidAmount,
    #[cfg_attr(feature = "serde", serde(rename = "bid_psbt_bad_signature"))]
    BadSignature,
    #[cfg_attr(feature = "serde", serde(rename = "bid_psbt_output_spent"))]
    OutputSpent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bytes(Vec<u8>);

impl Bytes {
    pub fn new(bytes: Vec<u8>) -> Self {
        Bytes(bytes)
    }
    pub fn to_vec(self) -> Vec<u8> {
        self.0
    }
    pub fn as_slice(&self) -> &[u8] {
        self.0.as_slice()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(feature = "serde")]
pub mod serde_bytes_impl {
    use bitcoin::hex::prelude::*;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use crate::Bytes;

    impl Serialize for Bytes {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            if serializer.is_human_readable() {
                serializer.serialize_str(&self.0.to_lower_hex_string())
            } else {
                serializer.serialize_bytes(&self.0)
            }
        }
    }

    impl<'de> Deserialize<'de> for Bytes {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            if deserializer.is_human_readable() {
                let hex_str = String::deserialize(deserializer)?;
                let c = Vec::from_hex(&hex_str).map_err(serde::de::Error::custom)?;
                Ok(Bytes::new(c))
            } else {
                let bytes: Vec<u8> = Deserialize::deserialize(deserializer)?;
                Ok(Bytes(bytes))
            }
        }
    }
}

#[cfg(feature = "bincode")]
pub mod bincode_bytes_impl {
    use alloc::vec::Vec;

    use bincode::{
        de::Decoder,
        enc::Encoder,
        error::{DecodeError, EncodeError},
        impl_borrow_decode, Decode, Encode,
    };

    use super::Bytes;

    impl Encode for Bytes {
        fn encode<E: Encoder>(&self, encoder: &mut E) -> Result<(), EncodeError> {
            Encode::encode(&self.as_slice(), encoder)
        }
    }

    impl Decode for Bytes {
        fn decode<D: Decoder>(decoder: &mut D) -> Result<Self, DecodeError> {
            let raw: Vec<u8> = Decode::decode(decoder)?;
            Ok(Bytes::new(raw))
        }
    }

    impl_borrow_decode!(Bytes);
}

impl Space {
    pub fn is_expired(&self, height: u32) -> bool {
        match self.covenant {
            Covenant::Transfer { expire_height, .. } => expire_height < height,
            _ => return false,
        }
    }

    pub fn is_owned(&self) -> bool {
        return matches!(self.covenant, Covenant::Transfer { .. });
    }

    pub fn claim_height(&self) -> Option<u32> {
        match &self.covenant {
            Covenant::Bid { claim_height, .. } => claim_height.clone(),
            _ => None,
        }
    }

    pub fn is_bid_spend(&self, tx_version: Version, txin: &TxIn) -> bool {
        if tx_version != BID_PSBT_TX_VERSION
            || txin.sequence != BID_PSBT_INPUT_SEQUENCE
            || txin.witness.len() != 1
            || txin.witness[0].len() != 65
            || txin.witness[0][64] != TapSighashType::SinglePlusAnyoneCanPay as u8
        {
            return false;
        }

        match &self.covenant {
            Covenant::Bid { signature, .. } => &txin.witness[0][..64] == signature.as_ref(),
            _ => false,
        }
    }

    pub fn data(&self) -> Option<&[u8]> {
        match &self.covenant {
            Covenant::Transfer { data, .. } => match &data {
                None => None,
                Some(data) => Some(data.as_slice()),
            },
            _ => None,
        }
    }

    pub fn data_owned(&self) -> Option<Bytes> {
        match &self.covenant {
            Covenant::Transfer { data, .. } => match &data {
                None => None,
                Some(data) => Some(data.clone()),
            },
            _ => None,
        }
    }
}

impl FullSpaceOut {
    pub fn outpoint(&self) -> OutPoint {
        OutPoint {
            txid: self.txid,
            vout: self.spaceout.n as u32,
        }
    }

    pub fn verify_bid_sig(&self) -> bool {
        if !self.spaceout.script_pubkey.is_p2tr() {
            return false;
        }

        let (mut tx, prevout, signature) = match self.refund_signing_info() {
            None => return false,
            Some(signing) => signing,
        };

        let mut sighash_cache = SighashCache::new(&mut tx);

        let sighash = match sighash_cache.taproot_key_spend_signature_hash(
            0,
            &prevout,
            TapSighashType::SinglePlusAnyoneCanPay,
        ) {
            Ok(sighash) => sighash,
            Err(_) => return false,
        };

        let msg = match Message::from_digest_slice(sighash.as_ref()) {
            Ok(msg) => msg,
            Err(_) => return false,
        };

        let ctx = bitcoin::secp256k1::Secp256k1::verification_only();

        let script_bytes = self.spaceout.script_pubkey.as_bytes();

        let pubkey = match bitcoin::XOnlyPublicKey::from_slice(&script_bytes[2..]) {
            Ok(pubkey) => pubkey,
            Err(_) => return false,
        };

        let schnorr_sig = match schnorr::Signature::from_slice(signature.as_ref()) {
            Ok(sig) => sig,
            Err(_) => return false,
        };

        ctx.verify_schnorr(&schnorr_sig, &msg, &pubkey).is_ok()
    }

    pub fn refund_signing_info(
        &self,
    ) -> Option<(Transaction, Prevouts<TxOut>, schnorr::Signature)> {
        if self.spaceout.space.is_none() {
            return None;
        }

        match &self.spaceout.space.as_ref().unwrap().covenant {
            Covenant::Bid {
                total_burned,
                signature,
                ..
            } => {
                let refund_amount = self.spaceout.value + *total_burned;
                Some((
                    Self::bid_psbt_tx(self, refund_amount, signature),
                    Prevouts::One(
                        0,
                        TxOut {
                            value: self.spaceout.value,
                            script_pubkey: self.spaceout.script_pubkey.clone(),
                        },
                    ),
                    signature.clone(),
                ))
            }
            _ => None,
        }
    }

    pub fn refund_psbt_data(&self) -> Option<(psbt::Input, TxOut)> {
        if self.spaceout.space.is_none() {
            return None;
        }

        match &self.spaceout.space.as_ref().unwrap().covenant {
            Covenant::Bid {
                total_burned,
                signature,
                ..
            } => {
                let refund_amount = self.spaceout.value + *total_burned;
                let mut witness = Witness::default();
                witness.push(
                    taproot::Signature {
                        signature: signature.clone(),
                        sighash_type: TapSighashType::SinglePlusAnyoneCanPay,
                    }
                    .to_vec(),
                );

                let refund_txout = TxOut {
                    value: refund_amount,
                    script_pubkey: self.spaceout.script_pubkey.clone(),
                };

                let input = psbt::Input {
                    witness_utxo: Some(TxOut {
                        value: self.spaceout.value,
                        script_pubkey: self.spaceout.script_pubkey.clone(),
                    }),
                    final_script_witness: Some(witness),
                    ..Default::default()
                };

                Some((input, refund_txout))
            }
            _ => None,
        }
    }

    fn bid_psbt_tx(
        auctioned_utxo: &FullSpaceOut,
        refund_amount: Amount,
        signature: &schnorr::Signature,
    ) -> Transaction {
        let mut witness = Witness::default();
        witness.push(
            taproot::Signature {
                signature: signature.clone(),
                sighash_type: TapSighashType::SinglePlusAnyoneCanPay,
            }
            .to_vec(),
        );

        let tx = Transaction {
            version: BID_PSBT_TX_VERSION,
            lock_time: BID_PSBT_TX_LOCK_TIME,
            input: vec![TxIn {
                previous_output: OutPoint {
                    txid: auctioned_utxo.txid,
                    vout: auctioned_utxo.spaceout.n as u32,
                },
                sequence: BID_PSBT_INPUT_SEQUENCE,
                witness,
                ..Default::default()
            }],
            output: vec![TxOut {
                value: refund_amount,
                script_pubkey: auctioned_utxo.spaceout.script_pubkey.clone(),
            }],
        };
        tx
    }
}
