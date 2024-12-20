use anyhow::Result;
use deopenchat_zkcircuit::{GUEST_CODE_FOR_ZK_PROOF_ELF, GUEST_CODE_FOR_ZK_PROOF_ID};
use common::Input;
use risc0_zkvm::{default_prover, ExecutorEnv, ProveInfo, ProverOpts};

pub fn prove(input: Input) -> Result<ProveInfo> {
    let env = ExecutorEnv::builder()
        .write(&input)?
        .build()?;

    let prover = default_prover();
    let prover_opts = ProverOpts::groth16();

    let prove_info = prover
        .prove_with_opts(env, GUEST_CODE_FOR_ZK_PROOF_ELF, &prover_opts)?;

    prove_info.receipt.verify(GUEST_CODE_FOR_ZK_PROOF_ID)?;
    Ok(prove_info)
}