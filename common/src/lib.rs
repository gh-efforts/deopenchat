use std::collections::HashMap;
use serde::{Deserialize, Serialize};

pub const SIGNATURE_SIZE: usize = 64;
pub const PUBLIC_KEY_SIZE: usize = 32;

pub const CLAIM_SIZE: usize = PUBLIC_KEY_SIZE + 4 + 4 + 8;

pub type PublicKey = [u8; PUBLIC_KEY_SIZE];

#[derive(Serialize, Deserialize, Copy, Clone)]
pub struct RequestMsg {
    pub seq: u32,
}

impl From<RequestMsg> for [u8; 4] {
    fn from(m: RequestMsg) -> Self {
        m.seq.to_be_bytes()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Request {
    pub msg: RequestMsg,
    pub signature: Vec<u8>,
}

#[derive(Serialize, Deserialize, Copy, Clone)]
pub struct ConfirmMsg {
    pub seq: u32,
    pub input_tokens: u32,
    pub resp_tokens: u32
}

impl From<ConfirmMsg> for [u8; 12] {
    fn from(m: ConfirmMsg) -> Self {
        let mut out = [0u8; 12];
        out[..4].copy_from_slice(&m.seq.to_be_bytes());
        out[4..8].copy_from_slice(&m.input_tokens.to_be_bytes());
        out[8..].copy_from_slice(&m.resp_tokens.to_be_bytes());
        out
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Confirm {
    pub msg: ConfirmMsg,
    pub signature: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
pub struct Round {
    pub request: Request,
    pub confirm: Confirm,
}

#[derive(Serialize, Deserialize)]
pub struct Input {
    pub rounds: HashMap<PublicKey, Vec<Round>>
}

#[derive(Debug)]
pub struct Claim {
    pub pk: PublicKey,
    pub start_seq: u32,
    pub rounds: u32,
    pub tokens_consumed: u64,
}

impl From<Claim> for [u8; CLAIM_SIZE] {
    fn from(claim: Claim) -> Self {
        let mut out: [u8; CLAIM_SIZE] = [0u8; CLAIM_SIZE];
        let (pk, buff) = out.split_at_mut(PUBLIC_KEY_SIZE);
        pk.copy_from_slice(&claim.pk);

        let (start_seq, buff) = buff.split_at_mut(4);
        start_seq.copy_from_slice(&claim.start_seq.to_be_bytes());

        let (rounds, tokens_consumed) = buff.split_at_mut(4);
        rounds.copy_from_slice(&claim.rounds.to_be_bytes());

        tokens_consumed.copy_from_slice(&claim.tokens_consumed.to_be_bytes());
        out
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompletionsReq<Req> {
    pub pk: PublicKey,
    pub raw_req: Req,
    pub request: Request,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CompletionsResp<Resp> {
    pub raw_response: Resp,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ConfirmReq {
    pub pk: PublicKey,
    pub confirm: Confirm
}