use ed25519_dalek::{ed25519, Verifier, VerifyingKey};
use common::{Claim, Input, PUBLIC_KEY_SIZE};
use risc0_zkvm::guest::env;

fn main() {
    let input: Input = env::read();
    let mut claims: Vec<u8> = Vec::with_capacity(input.rounds.len() * (PUBLIC_KEY_SIZE + 4 + 4 + 8));

    for (client_pk, rounds) in &input.rounds {
        let mut curr_seq = rounds[0].request.msg.seq;
        let vk = VerifyingKey::from_bytes(client_pk).expect("invalid pk");

        let mut tokens_consumed: u64 = 0;

        for round in rounds {
            assert_eq!(curr_seq, round.request.msg.seq);
            assert_eq!(curr_seq, round.confirm.msg.seq);

            let signature = ed25519::Signature::from_slice(&round.request.signature).expect("invalid signature");
            let msg = <[u8; 4]>::from(round.request.msg);
            assert!(vk.verify(&msg, &signature).is_ok());

            let signature = ed25519::Signature::from_slice(&round.confirm.signature).expect("invalid signature");
            let msg = <[u8; 12]>::from(round.confirm.msg);
            assert!(vk.verify(&msg, &signature).is_ok());

            tokens_consumed += round.confirm.msg.input_tokens as u64;
            tokens_consumed += round.confirm.msg.resp_tokens as u64;
            curr_seq += 1;
        }

        let claim = Claim {
            pk: *client_pk,
            start_seq: rounds[0].request.msg.seq,
            rounds: rounds.len() as u32,
            tokens_consumed
        };

        let claim_buf: [u8; 48] = claim.into();
        claims.extend_from_slice(&claim_buf);
    }

    env::commit_slice(&claims);
}
