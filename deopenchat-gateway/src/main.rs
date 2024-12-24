use crate::metadata::{MetadataCache, PeerStatus, RoundState};
use alloy::network::EthereumWallet;
use alloy::primitives::{Address, Bytes, FixedBytes};
use alloy::providers::{Provider, ProviderBuilder, WalletProvider};
use alloy::signers::local::PrivateKeySigner;
use alloy::sol;
use alloy::transports::http::reqwest::Url;
use alloy::transports::Transport;
use anyhow::{anyhow, ensure, Result};
use async_openai::config::OpenAIConfig;
use axum::body::Body;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::{Parser, Subcommand};
use common::{CompletionsReq, CompletionsResp, ConfirmReq, Input, PublicKey, Round, CLAIM_SIZE};
use ed25519_dalek::{ed25519, Verifier, VerifyingKey};
use futures_util::TryFutureExt;
use log::{info, LevelFilter};
use log4rs::append::console::ConsoleAppender;
use log4rs::config::{Appender, Root};
use log4rs::encode::pattern::PatternEncoder;
use std::collections::HashMap;
use std::future::IntoFuture;
use std::net::SocketAddr;
use std::process::ExitCode;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

mod metadata;

sol!{
    #[sol(rpc)]
    "../contract/src/Deopenchat.sol"
}

fn logger_init() -> anyhow::Result<()> {
    let log_level = LevelFilter::from_str(
        std::env::var("DEOPENCHAT_GATEWAY_LOG").as_deref().unwrap_or("INFO"),
    )?;

    let pattern = if log_level >= LevelFilter::Debug {
        "[{d(%Y-%m-%d %H:%M:%S)}] {h({l})} {f}:{L} - {m}{n}"
    } else {
        "[{d(%Y-%m-%d %H:%M:%S)}] {h({l})} {t} - {m}{n}"
    };

    let stdout = ConsoleAppender::builder()
        .encoder(Box::new(PatternEncoder::new(pattern)))
        .build();

    let config = log4rs::Config::builder()
        .appender(Appender::builder().build("stdout", Box::new(stdout)))
        .build(
            Root::builder()
                .appender("stdout")
                .build(log_level),
        )?;

    log4rs::init_config(config)?;
    Ok(())
}

struct Context<P> {
    md_cache: MetadataCache,
    alloy_provider: P,
    provider_address: Address,
    deopenchat_contact_address: Address,
    backend_client: async_openai::Client<OpenAIConfig>,
    accumulated_tokens: AtomicU64
}

async fn completions<T, P>(
    State(ctx): State<Arc<Context<P>>>,
    Json(req): Json<CompletionsReq<async_openai::types::CreateCompletionRequest>>
) -> Response
    where
        T: Send + Sync + Transport + Clone,
        P: Provider<T> + 'static
{
    let fut = async {
        let vk = VerifyingKey::from_bytes(&req.pk)?;
        let signature= ed25519::Signature::from_slice(&req.request.signature)?;
        vk.verify(&<[u8; 4]>::from(req.request.msg), &signature)?;

        let deopenchat = Deopenchat::new(ctx.deopenchat_contact_address, &ctx.alloy_provider);
        let builder = deopenchat.viewStatus(ctx.provider_address, FixedBytes::new(req.pk));
        let record= builder.call().await?._0;

        anyhow::ensure!(record.remainingTokens > 0);
        ctx.md_cache.req(&req).await?;

        let resp = ctx.backend_client
            .completions()
            .create(req.raw_req.clone())
            .await?;

        let cr = CompletionsResp {
            raw_response: resp,
        };

        ctx.md_cache.resp(&req, &cr).await?;
        Ok(cr)
    };

    match fut.await {
        Ok(resp) => {
            let ret = serde_json::to_vec(&resp).unwrap();
            Response::new(Body::from(ret))
        }
        Err(e) => {
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from(e.to_string()))
                .unwrap()
        }
    }
}

async fn completions_confirm<T, P>(
    State(ctx): State<Arc<Context<P>>>,
    Json(req): Json<ConfirmReq>
) -> Response
    where
        T: Send + Sync + Transport + Clone,
        P: Provider<T> + 'static
{
    let fut = async {
        let vk = VerifyingKey::from_bytes(&req.pk)?;
        let signature= ed25519::Signature::from_slice(&req.confirm.signature)?;
        vk.verify(&<[u8; 12]>::from(req.confirm.msg), &signature)?;

        let rd = ctx.md_cache.load_round(req.pk, req.confirm.msg.seq).await?;
        let usage = rd.resp.raw_response.usage.ok_or_else(|| anyhow!("missing usage"))?;

        ensure!(req.confirm.msg.input_tokens >= usage.prompt_tokens);
        ensure!(req.confirm.msg.resp_tokens >= usage.completion_tokens);

        ctx.md_cache.confirm(&req).await?;
        ctx.accumulated_tokens.fetch_add(req.confirm.msg.input_tokens as u64 + req.confirm.msg.resp_tokens as u64, Ordering::Relaxed);
        Ok(())
    };

    match fut.await {
        Ok(resp) => {
            let ret = serde_json::to_vec(&resp).unwrap();
            Response::new(Body::from(ret))
        }
        Err(e) => {
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from(e.to_string()))
                .unwrap()
        }
    }
}

async fn current_seq<T, P>(
    State(ctx): State<Arc<Context<P>>>,
    axum::extract::Path(pk_str): axum::extract::Path<String>
) -> Response
    where
        T: Send + Sync + Transport + Clone,
        P: Provider<T> + 'static
{
    let fut = async  {
        let pk: PublicKey = hex::decode(pk_str)?.as_slice().try_into()?;
        let status_opt = ctx.md_cache.load_status(pk).await?;

        let status = match status_opt {
            Some(s) => s,
            None => {
                let deopenchat = Deopenchat::new(ctx.deopenchat_contact_address, &ctx.alloy_provider);
                let status = deopenchat.viewStatus(ctx.provider_address, FixedBytes::new(pk))
                    .call()
                    .await?._0;

                PeerStatus {
                    seq: status.seq,
                    commit_seq: status.seq,
                    state: RoundState::Completed
                }
            }
        };

        Ok::<_, anyhow::Error>(status.seq)
    };

    match fut.await {
        Ok(seq) => {
            Response::new(Body::from(seq.to_string()))
        }
        Err(e) => {
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from(e.to_string()))
                .unwrap()
        }
    }
}

async fn commit_handler<T, P> (
    ctx: Arc<Context<P>>,
    commit_high_water_level: u64
) -> Result<()>
    where
        T: Send + Sync + Transport + Clone,
        P: Provider<T> + 'static
{
    let prover_image_id = deopenchat_prover::image_id();
    info!("prover image id: {}", prover_image_id);

    let deopenchat = Deopenchat::new(ctx.deopenchat_contact_address, &ctx.alloy_provider);
    let contact_image_id = deopenchat.getImageId()
        .call()
        .await?
        ._0
        .0;

    ensure!(prover_image_id.as_bytes() == contact_image_id, "contact image id mismatch, expected: {}, got: {}", prover_image_id, hex::encode(contact_image_id));

    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

        if ctx.accumulated_tokens.load(Ordering::Relaxed) < commit_high_water_level {
            continue;
        }

        let mapping = ctx.md_cache.load_all_history().await?;
        let rounds = mapping.into_iter()
            .map(|(k, rounds)| {
                let rounds= rounds.into_iter()
                    .filter(|r| r.confirm_msg.is_some())
                    .map(|round| {
                        Round {
                            request: round.req.request,
                            confirm: round.confirm_msg.unwrap().confirm
                        }
                    })
                    .collect::<Vec<_>>();
                (k, rounds)
            })
            .collect::<HashMap<_, _>>();

        let input = Input {
            rounds
        };

        let prove_info = tokio::task::spawn_blocking(move || {
            deopenchat_prover::prove(input)
        }).await??;

        let seal = risc0_ethereum_contracts::encode_seal(&prove_info.receipt)?;
        let journal = prove_info.receipt.journal.bytes;

        let mut buff = journal.as_slice();
        let mut claims = Vec::new();

        while buff.len() > 0 {
            let (claim_buf, r) = journal.split_at(CLAIM_SIZE);
            buff = r;

            let claim = Deopenchat::Claim {
                clientPk: FixedBytes::new((&claim_buf[..32]).try_into().unwrap()),
                seq: u32::from_be_bytes((&claim_buf[32..36]).try_into().unwrap()),
                rounds: u32::from_be_bytes((&claim_buf[36..40]).try_into().unwrap()),
                numberTokensConsumed: u64::from_be_bytes((&claim_buf[40..48]).try_into().unwrap()),
            };
            claims.push(claim);
        }

        let commit_claims = claims.iter()
            .map(|c| {
                common::Claim {
                    pk: c.clientPk.0,
                    start_seq: c.seq,
                    rounds: c.rounds,
                    tokens_consumed: c.numberTokensConsumed,
                }
            })
            .collect::<Vec<_>>();

        let tx = deopenchat.claim(claims, Bytes::from(seal))
            .send()
            .await?
            .watch()
            .await?;

        info!("claim TX: {}", tx);

        ctx.md_cache.commit(&commit_claims).await?;
    }
}

async fn provider_register(
    chain_endpoint: Url,
    deopenchat_contact_address: Address,
    wallet_sk: &str,
    ktokens_cost: u32,
    endpoint: String,
    model: String,
) -> Result<()> {
    let signer = PrivateKeySigner::from_str(wallet_sk)?;
    let wallet = EthereumWallet::from(signer);

    let alloy_provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(wallet)
        .on_http(chain_endpoint);

    let deopenchat = Deopenchat::new(deopenchat_contact_address, alloy_provider);

    let tx = deopenchat.providerRegister(
        ktokens_cost,
        endpoint,
        model
    )
    .send()
    .await?
    .watch()
    .await?;

    info!("provider register TX: {}", tx);
    Ok(())
}

async fn daemon(
    bind_addr: SocketAddr,
    backend_api: String,
    wallet_sk: &str,
    chain_endpoint: Url,
    deopenchat_contact_address: Address,
    commit_high_water_level: u64
) -> Result<()> {
    let openai_config = OpenAIConfig::new().with_api_base(backend_api);
    let backend = async_openai::Client::with_config(openai_config);

    let signer = PrivateKeySigner::from_str(wallet_sk)?;
    let wallet = EthereumWallet::from(signer);

    let alloy_provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(wallet)
        .on_http(chain_endpoint);

    let provider_address = alloy_provider.default_signer_address();

    let md_cache = MetadataCache::new(&std::env::current_dir()?.join("cache"));

    let ctx = Arc::new(Context {
        md_cache,
        alloy_provider,
        provider_address,
        deopenchat_contact_address,
        backend_client: backend,
        accumulated_tokens: AtomicU64::new(0)
    });

    let commit_handler_fut = async {
        tokio::spawn(commit_handler(
            ctx.clone(),
            commit_high_water_level
        )).await?
    };

    let app = Router::new()
        .route("/v1/completions", get(completions))
        .route("/v1/completions/confirm", post(completions_confirm))
        .route("/v1/completions/seq/:pk", get(current_seq))
        .with_state(ctx.clone());

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    info!("Listening on http://{}", bind_addr);
    let axum_fut = axum::serve(listener, app).into_future().map_err(|e| anyhow!(e));

    tokio::try_join!(commit_handler_fut, axum_fut)?;
    Ok(())
}

#[derive(Subcommand)]
enum SubCommand {
    Daemon {
        #[arg(long)]
        bind_addr: SocketAddr,

        #[arg(long)]
        backend_api: String,

        #[arg(long)]
        commit_high_water_level: u64
    },
    ProviderRegister {
        #[arg(long)]
        ktokens_cost: u32,

        #[arg(long)]
        endpoint: String,

        #[arg(long)]
        model: String,
    }
}

#[derive(Parser)]
#[command(version)]
struct Args {
    #[arg(short, long)]
    chain_endpoint: Url,

    #[arg(short, long)]
    deopenchat_contact_address: Address,

    #[arg(long)]
    wallet_sk: String,

    #[command(subcommand)]
    cmd: SubCommand
}

fn exec(args: Args) -> Result<()> {
    logger_init()?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    match args.cmd {
        SubCommand::Daemon {
            bind_addr,
            backend_api,
            commit_high_water_level,
        } => {
            rt.block_on(daemon(
                bind_addr,
                backend_api,
                &args.wallet_sk,
                args.chain_endpoint,
                args.deopenchat_contact_address,
                commit_high_water_level
            ))
        }
        SubCommand::ProviderRegister {
            ktokens_cost,
            endpoint,
            model
        } => {
            rt.block_on(provider_register(
                args.chain_endpoint,
                args.deopenchat_contact_address,
                &args.wallet_sk,
                ktokens_cost,
                endpoint,
                model
            ))
        }
    }
}

fn main() -> ExitCode {
    let args = Args::parse();

    match exec(args) {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{:?}", e);
            ExitCode::FAILURE
        }
    }
}
