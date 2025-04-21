# Deopenchat

## Project Summary

This project creates a decentralized marketplace for AI model inference services, enabling Filecoin Storage Providers (SPs) to monetize their idle compute machines post-data encapsulation. By repurposing these underutilized resources, SPs can offer inference services compatible with OpenAI API standards, while clients access these services through a token-based payment system. The platform leverages blockchain smart contracts to manage provider registration, API call tracking, and fee settlement, ensuring transparency, security, and trust. The integration with Filecoin’s ecosystem begins with utilizing SPs’ idle compute power for API forwarding and inference tasks, with future phases incorporating storage and distributed training capabilities to further enhance SP revenue and platform functionality.



## Key Components

### Roles

- **Service** **Provider (Filecoin SP)**:
  - Utilizes idle compute machines (post-Filecoin data encapsulation) to host model inference services compatible with OpenAI API standards.
  - Initially acts as an API forwarder, redirecting client requests to inference models, and later provides dedicated compute resources for inference tasks.
  - Earns revenue based on the number of tokens processed per API call, monetizing otherwise idle hardware.
- **Client**:
  - Initiates API requests to SPs’ inference services, leveraging Filecoin’s distributed compute network.
  - Pays fees based on token consumption, managed through smart contracts.
- **Smart Contract**:
  - Manages SP registration, including details like SP address, API endpoint, compute capacity, and pricing strategy.
  - Tracks API call transactions, including token consumption and fees, with provisions for future storage integration (e.g., Content Identifiers).
  - Automates fee settlement, compensating SPs for compute resource usage.
- **Filecoin Compute Network**:
  - Comprises SPs’ idle compute machines, orchestrated to handle model inference tasks.
  - Supports phased integration, starting with API forwarding, progressing to compute resource provision, and planning for storage and distributed training.



## Workflow

1. **Service** **Provider (SP) Registration**:
   1. SPs register their idle compute machines on the platform, specifying API endpoints, available compute capacity, and pricing strategies.
   2. Initially, SPs deploy lightweight API forwarding services to route client requests to inference models hosted on their machines or external compute nodes.
   3. Registration details are recorded on the smart contract, including SP identity and compute metadata.
2. **Client Token Purchase**:
   1. Clients transfer on-chain funds to the smart contract, specifying an SP to purchase tokens at the SP’s rate.
   2. Clients generate a public-private key pair, uploading the public key to the contract.
   3. The contract updates a mapping: SP A -> Client A -> (public key, transaction sequence, remaining tokens, token price).
3. **Client Request Initiation**:
   1. Clients construct a signed request query_seq = (msg, sign(msg, sk)), where msg includes sender, sequence number, content, and maximum tokens. The sequence number increments with each request.
   2. The request is sent off-chain to the SP’s API endpoint, which either forwards it to an inference model (Phase 1) or processes it directly on the SP’s compute resources (Phase 2).
   3. Upon receiving the response resp_seq, clients send confirm_seq = {seq, sign(resp_seq, sk)} to complete the round.
4. **SP Request Handling**:
   1. SPs verify the request’s legitimacy using the client’s public key and ensure sequence number continuity.
   2. In **Phase 1 (API Forwarding)**, SPs route the request to an external or hosted inference model, acting as intermediaries.
   3. In **Phase 2 (Compute Provision)**, SPs execute the inference task directly on their idle compute machines, leveraging local resources for model inference.
   4. SPs confirm sufficient client tokens, send resp_seq to the client, and await confirm_seq to finalize the round.
5. **Fee Claim by SP**:
   1. After completing at least one round, SPs construct a proof including the round sequence, total tokens consumed, and signature verifications.
   2. The proof, along with a hash of the request and response messages, is submitted to the contract.
   3. The contract verifies the proof and disburses fees, rewarding SPs for compute resource usage.



## Usage

clone project

```shell
https://github.com/gh-efforts/deopenchat.git
```

install rust-toolchain

```shell
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```



### Start deopenchat-gateway

gateway is a service used to expose the provider's API to the outside world.

1. build gateway

   ```shell
   cd deopenchat-gateway
   cargo build --release
   ```

2. service provider registration

   ```shell
   cd ../target/release
   ```

   ```shell
   ./deopenchat-gateway --chain-endpoint <CHAIN_ENDPOINT> --deopenchat-contact-address <DEOPENCHAT_CONTACT_ADDRESS> --wallet-sk <WALLET_SK> provider-register --ktokens-cost <KTOKENS_COST> --endpoint <ENDPOINT> --model <MODEL>
   ```

3. start gateway

   ```shell
   ./deopenchat-gateway --chain-endpoint <CHAIN_ENDPOINT> --deopenchat-contact-address <DEOPENCHAT_CONTACT_ADDRESS> --wallet-sk <WALLET_SK> daemon --bind-addr <BIND_ADDR> --backend-api <BACKEND_API> --commit-high-water-level <COMMIT_HIGH_WATER_LEVEL>
   ```



### Start deopenchat-bridge
Bridge is used to establish a connection with a provider and provide OpenAI interface services locally.

1. build bridge

   ```shell
   cd deopenchat-bridge
   cargo build --release
   ```

2. fetch tokens

	```shell
   ./deopenchat-bridge --chain-endpoint <CHAIN_ENDPOINT> --deopenchat-contact-address <DEOPENCHAT_CONTACT_ADDRESS> fetch-tokens --provider <PROVIDER> --client-pk <CLIENT_PK> --eth-wallet-sk <ETH_WALLET_SK> --ktokens <KTOKENS>
   ```

3. start bridge

   ```shell
   ./deopenchat-bridge --chain-endpoint <CHAIN_ENDPOINT> --deopenchat-contact-address <DEOPENCHAT_CONTACT_ADDRESS> daemon --bind-addr <BIND_ADDR> --provider <PROVIDER> --client-sk <CLIENT_SK>
   ```

   
