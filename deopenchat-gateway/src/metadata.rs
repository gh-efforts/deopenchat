use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use common::{CompletionsReq, CompletionsResp, ConfirmReq, PublicKey};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

#[derive(Copy, Clone, PartialEq, Serialize, Deserialize)]
pub enum RoundState {
    Requested,
    WaitingConfirm,
    Completed
}

#[derive(Copy, Clone, PartialEq, Serialize, Deserialize)]
pub struct PeerStatus {
    pub seq: u32,
    pub commit_seq: u32,
    pub state: RoundState,
}

#[derive(Serialize, Deserialize)]
pub struct RoundData {
    pub seq: u32,
    pub req: CompletionsReq<async_openai::types::CreateCompletionRequest>,
    pub resp: CompletionsResp<async_openai::types::CreateCompletionResponse>,
    pub confirm_msg: Option<ConfirmReq>,
}

pub struct MetadataCache {
    round_status_dir: PathBuf,
    history_dir: PathBuf,
    locks: std::sync::Mutex<HashMap<PublicKey, Arc<tokio::sync::RwLock<()>>>>
}

impl MetadataCache {
    pub fn new(cache_dir: &Path) -> Self {
        Self {
            round_status_dir: cache_dir.join("status"),
            history_dir: cache_dir.join("history"),
            locks: std::sync::Mutex::new(HashMap::new())
        }
    }

    pub async fn req(&self, req: &CompletionsReq<async_openai::types::CreateCompletionRequest>) -> Result<()> {
        let key = req.pk;
        let key_str = hex::encode(&key);

        let lock= {
            let mut lg = self.locks.lock().unwrap();
            lg.entry(key).or_insert_with(|| Arc::new(tokio::sync::RwLock::new(()))).clone()
        };

        let _lg = lock.write().await;
        let buf = match cacache::read(&self.round_status_dir, &key_str).await {
            Ok(buf) => buf,
            Err(cacache::Error::EntryNotFound(_, _)) => {
                ensure!(req.request.msg.seq == 1);

                let round = PeerStatus {
                    seq: 1,
                    commit_seq: 0,
                    state: RoundState::Requested,
                };

                let buf = serde_json::to_vec(&round)?;
                cacache::write(&self.round_status_dir, key_str, &buf).await?;
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        };

        let curr: PeerStatus = serde_json::from_slice(&buf)?;
        ensure!(curr.state == RoundState::Completed);
        ensure!(curr.seq + 1 == req.request.msg.seq);

        let round = PeerStatus {
            seq: req.request.msg.seq,
            commit_seq: curr.commit_seq,
            state: RoundState::Requested,
        };

        let buf = serde_json::to_vec(&round)?;
        cacache::write(&self.round_status_dir, key_str, &buf).await?;
        Ok(())
    }

    pub async fn resp(
        &self,
        req: &CompletionsReq<async_openai::types::CreateCompletionRequest>,
        resp: &CompletionsResp<async_openai::types::CreateCompletionResponse>
    ) -> Result<()> {
        let key = req.pk;
        let key_str = hex::encode(&key);

        let lock= {
            let mut lg = self.locks.lock().unwrap();
            lg.entry(key).or_insert_with(|| Arc::new(tokio::sync::RwLock::new(()))).clone()
        };

        let _guard = lock.write().await;

        let buf = cacache::read(&self.round_status_dir, &key_str).await?;
        let mut curr_round: PeerStatus = serde_json::from_slice(&buf)?;

        ensure!(curr_round.state == RoundState::Requested);
        ensure!(curr_round.seq == req.request.msg.seq);

        let rd = RoundData {
            seq: req.request.msg.seq,
            req: req.clone(),
            resp: resp.clone(),
            confirm_msg: None,
        };

        cacache::write(&self.history_dir, format!("{}-{}", key_str, curr_round.seq), &serde_json::to_vec(&rd)?).await?;

        curr_round.state = RoundState::WaitingConfirm;
        cacache::write(&self.round_status_dir, key_str, &serde_json::to_vec(&curr_round)?).await?;
        Ok(())
    }

    pub async fn confirm(&self, confirm: &ConfirmReq) -> Result<()> {
        let key = confirm.pk;
        let key_str = hex::encode(&key);

        // todo
        let lock= {
            let mut lg = self.locks.lock().unwrap();
            lg.entry(key).or_insert_with(|| Arc::new(tokio::sync::RwLock::new(()))).clone()
        };

        let _guard = lock.write().await;

        let buf = cacache::read(&self.round_status_dir, &key_str).await?;
        let mut curr_round: PeerStatus = serde_json::from_slice(&buf)?;
        ensure!(curr_round.state == RoundState::WaitingConfirm);
        ensure!(curr_round.seq == confirm.confirm.msg.seq);

        let buf = cacache::read(&self.history_dir, &format!("{}-{}", key_str, confirm.confirm.msg.seq)).await?;
        let mut rd: RoundData = serde_json::from_slice(&buf)?;
        rd.confirm_msg = Some(confirm.clone());
        cacache::write(&self.history_dir, &format!("{}-{}", key_str, curr_round.seq), &serde_json::to_vec(&rd)?).await?;

        curr_round.state = RoundState::Completed;
        cacache::write(&self.round_status_dir, key_str, &serde_json::to_vec(&curr_round)?).await?;
        Ok(())
    }

    pub async fn load_round(&self, key: PublicKey, seq: u32) -> Result<RoundData> {
        let key_str = hex::encode(&key);

        let lock= {
            let mut lg = self.locks.lock().unwrap();
            lg.entry(key).or_insert_with(|| Arc::new(tokio::sync::RwLock::new(()))).clone()
        };

        let _guard = lock.read().await;

        let buf = cacache::read(&self.history_dir, &format!("{}-{}", key_str, seq)).await?;
        let rd: RoundData = serde_json::from_slice(&buf)?;
        Ok(rd)
    }

    pub async fn load_status(&self, key: PublicKey) -> Result<Option<PeerStatus>> {
        let lock = {
            let lg = self.locks.lock().unwrap();
            match lg.get(&key) {
                None => return Ok(None),
                Some(l) => l.clone()
            }
        };

        let key_str = hex::encode(&key);
        let _guard = lock.read().await;

        let buf = cacache::read(&self.round_status_dir, &key_str).await?;
        let curr_round: PeerStatus = serde_json::from_slice(&buf)?;
        Ok(Some(curr_round))
    }

    pub async fn load_all_history(&self) -> Result<HashMap<PublicKey, Vec<RoundData>>> {
        let keys = self.locks.lock().unwrap().clone();
        let mut out = HashMap::new();

        for (key, lock) in keys {
            let key_str = hex::encode(&key);
            let _guard = lock.read().await;

            let buf = cacache::read(&self.round_status_dir, &key_str).await?;
            let s: PeerStatus = serde_json::from_slice(&buf)?;

            let mut rounds = Vec::new();

            for seq in s.commit_seq + 1..s.seq {
                let buf = cacache::read(&self.history_dir, &format!("{}-{}", key_str, seq)).await?;
                let rd: RoundData = serde_json::from_slice(&buf)?;
                rounds.push(rd);
            }

            if !rounds.is_empty() {
                out.insert(key, rounds);
            }
        }

        Ok(out)
    }

    pub async fn commit(&self, claims: &[common::Claim]) -> Result<()> {
        let keys = self.locks.lock().unwrap().clone();

        for claim in claims {
            let key_str = hex::encode(&claim.pk);

            let lock = keys.get(&claim.pk).ok_or_else(|| anyhow::anyhow!("no such lock"))?;
            let _guard = lock.write().await;

            let buf = cacache::read(&self.round_status_dir, &key_str).await?;
            let mut s: PeerStatus = serde_json::from_slice(&buf)?;

            ensure!(s.commit_seq + 1 == claim.start_seq);
            ensure!(s.seq >= s.commit_seq + claim.rounds);

            s.commit_seq += claim.rounds;
            cacache::write(&self.round_status_dir, &key_str, &serde_json::to_vec(&s)?).await?;

            for seq in claim.start_seq..claim.start_seq + claim.rounds {
                cacache::remove(&self.history_dir, &format!("{}-{}", key_str, seq)).await?;
            }
        }
        Ok(())
    }
}