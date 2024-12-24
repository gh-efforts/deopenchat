use alloy::hex::FromHex;
use alloy::network::EthereumWallet;
use alloy::primitives::{Address, FixedBytes, U256};
use alloy::providers::{ProviderBuilder};
use alloy::signers::local::PrivateKeySigner;
use alloy::{hex, sol};
use anyhow::{anyhow, Result};
use axum::body::Body;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::get;
use axum::{Json, Router};
use clap::{Parser, Subcommand};
use common::{CompletionsReq, CompletionsResp, Confirm, ConfirmMsg, ConfirmReq, Request, RequestMsg};
use ed25519_dalek::{SecretKey, Signer, SigningKey};
use futures_util::TryFutureExt;
use log::{error, info, LevelFilter};
use log4rs::append::console::ConsoleAppender;
use log4rs::config::{Appender, Root};
use log4rs::encode::pattern::PatternEncoder;
use prettytable::{row, Table};
use reqwest::Url;
use std::future::IntoFuture;
use std::net::SocketAddr;
use std::process::ExitCode;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::mpsc;

sol!{
    #[sol(rpc)]
    "../contract/src/Deopenchat.sol"
}

struct Context {
    task_sender: mpsc::Sender<(async_openai::types::CreateCompletionRequest, tokio::sync::oneshot::Sender<async_openai::types::CreateCompletionResponse>)>,
}

async fn completions_handler(
    client: reqwest::Client,
    endpoint: Url,
    mut seq: u32,
    sk: SigningKey,
    mut task_recv: mpsc::Receiver<(async_openai::types::CreateCompletionRequest, tokio::sync::oneshot::Sender<async_openai::types::CreateCompletionResponse>)>
) -> Result<()> {
    let completions_url = endpoint.join("/v1/completions")?;

    while let Some((req, tx)) = task_recv.recv().await {
        let msg = RequestMsg {
            seq
        };

        let signature = sk.sign(&<[u8; 4]>::from(msg));

        let req = CompletionsReq {
            pk: sk.verifying_key().to_bytes(),
            raw_req: req,
            request: Request {
                msg,
                signature: signature.to_vec()
            }
        };

        let resp = client.get(completions_url.clone())
            .json(&req)
            .send()
            .await?;

        let resp: CompletionsResp<async_openai::types::CreateCompletionResponse> = match resp.json().await {
            Err(e) => {
                error!("completions error: {:?}", e);
                return Err(anyhow!("completions failed"));
            }
            Ok(resp) => resp,
        };

        let usage = resp.raw_response.usage.clone().ok_or_else(|| anyhow!("missing usage"))?;
        // todo verify number of tokens

        let _ = tx.send(resp.raw_response);

        let msg = ConfirmMsg {
            seq,
            input_tokens: usage.prompt_tokens,
            resp_tokens: usage.completion_tokens
        };

        let signature = sk.sign(&<[u8; 12]>::from(msg));

        let req = ConfirmReq {
            pk: sk.verifying_key().to_bytes(),
            confirm: Confirm {
                msg,
                signature: signature.to_vec()
            }
        };

        let completions_confirm_url =  endpoint.join("/v1/completions/confirm")?;

        client.post(completions_confirm_url)
            .json(&req)
            .send()
            .await?;

        seq += 1;
    }

    Ok(())
}

async fn completions(
    State(ctx): State<Arc<Context>>,
    Json(req): Json<async_openai::types::CreateCompletionRequest>
) -> Response {
    let (oneshot_tx, oneshot_rx) = tokio::sync::oneshot::channel();

    let fut = async {
        ctx.task_sender.send((req, oneshot_tx)).await?;
        let resp= oneshot_rx.await?;
        Result::<_, anyhow::Error>::Ok(resp)
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

async fn daemon(
    bind_addr: SocketAddr,
    provider: Address,
    deopenchat_contact_address: Address,
    client_sk_str: &str,
    chain_endpoint: Url
) -> Result<()> {
    let client = reqwest::Client::new();

    let client_sk = SigningKey::from(SecretKey::from_hex(client_sk_str)?);
    let client_pk = client_sk.verifying_key();

    let alloy_provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .on_http(chain_endpoint);

    let provider_info = Deopenchat::new(deopenchat_contact_address, &alloy_provider)
        .getProvider(provider)
        .call()
        .await?
        ._0;

    let provider_endpoint: Url = provider_info.endpoint.parse()?;
    let query_seq = provider_endpoint
        .join("/v1/completions/seq/")?
        .join(&hex::encode(client_pk.to_bytes()))?;

    let seq_str = client.get(query_seq)
        .send()
        .await?
        .text()
        .await?;

    let seq = u32::from_str(&seq_str)?;

    let (task_tx, task_rx) = tokio::sync::mpsc::channel(64);

    let completions_fut = async {
        tokio::spawn(completions_handler(
            client,
            provider_info.endpoint.parse()?,
            seq + 1,
            client_sk,
            task_rx,
        )).await?
    };

    let ctx = Context {
        task_sender: task_tx
    };

    let app = Router::new()
        .route("/v1/completions", get(completions))
        .with_state(Arc::new(ctx));

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    info!("Listening on http://{}", bind_addr);
    let axum_fut = axum::serve(listener, app).into_future().map_err(|e| anyhow!(e));

    tokio::try_join!(completions_fut, axum_fut)?;
    Ok(())
}

async fn fetch_tokens(
    chain_endpoint: Url,
    deopenchat_contact_address: Address,
    wallet_sk: &str,
    provider: Address,
    ktokens: u32,
    client_key: &str
) -> Result<()> {
    let signer = PrivateKeySigner::from_str(wallet_sk)?;
    let wallet = EthereumWallet::from(signer);

    let alloy_provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(wallet)
        .on_http(chain_endpoint);

    let deopenchat = Deopenchat::new(deopenchat_contact_address, alloy_provider);

    let provider_info = deopenchat.getProvider(provider)
        .call()
        .await?;

    let amount = provider_info._0.costPerKTokens * ktokens;

    let tx = deopenchat.fethTokens(
        provider,
        ktokens,
        FixedBytes::new(hex::decode_to_array::<_, 32>(client_key)?)
    )
    .value(U256::from(amount))
    .send()
    .await?
    .watch()
    .await?;

    println!("fetch tokens watched: {:?}", tx);
    Ok(())
}

async fn print_all_providers(
    chain_endpoint: Url,
    deopenchat_contact_address: Address
) -> Result<()> {
    let alloy_provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .on_http(chain_endpoint);

    let deopenchat = Deopenchat::new(deopenchat_contact_address, alloy_provider);

    let providers = deopenchat.getAllProviders()
        .call()
        .await?;

    let mut table = Table::new();

    table.add_row(row!["ADDRESS", "COST_PER_KTOKENS", "ENDPOINT", "MODEL"]);

    for p in providers._0 {
        table.add_row(row![
            hex::encode(p.providerAddress),
            p.costPerKTokens,
            p.endpoint,
            p.model
        ]);
    }

    table.printstd();
    Ok(())
}

#[derive(Subcommand)]
enum SubCommand {
    Daemon {
        #[arg(short, long)]
        bind_addr: SocketAddr,

        #[arg(short, long)]
        provider: Address,

        #[arg(short, long)]
        client_sk: String,
    },
    FetchTokens {
        #[arg(short, long)]
        provider: Address,

        #[arg(short, long)]
        client_pk: String,

        #[arg(short, long)]
        eth_wallet_sk: String,

        #[arg(short, long)]
        ktokens: u32,
    },
    PrintAllProviders
}

#[derive(Parser)]
#[command(version)]
struct Args {
    #[arg(short, long)]
    chain_endpoint: Url,

    #[arg(short, long)]
    deopenchat_contact_address: Address,

    #[command(subcommand)]
    cmd: SubCommand
}

fn logger_init() -> anyhow::Result<()> {
    let log_level = LevelFilter::from_str(
        std::env::var("DEOPENCHAT_BRIDGE_LOG").as_deref().unwrap_or("INFO"),
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

fn exec(args: Args) -> Result<()> {
    logger_init()?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    match args.cmd {
        SubCommand::Daemon {
            bind_addr,
            provider,
            client_sk
        } => {
            rt.block_on(daemon(
                bind_addr,
                provider,
                args.deopenchat_contact_address,
                &client_sk,
                args.chain_endpoint
            ))
        }
        SubCommand::FetchTokens {
            provider,
            client_pk,
            eth_wallet_sk ,
            ktokens
        } => {
            rt.block_on(fetch_tokens(
                args.chain_endpoint,
                args.deopenchat_contact_address,
                &eth_wallet_sk,
                provider,
                ktokens,
                &client_pk
            ))
        }
        SubCommand::PrintAllProviders => {
            rt.block_on(print_all_providers(args.chain_endpoint, args.deopenchat_contact_address))
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
