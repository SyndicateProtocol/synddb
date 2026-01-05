# SyndDB Staging Deployment

This guide covers deploying SyndDB to Base Sepolia using GCP Confidential VMs.

## Prerequisites

- [Terraform](https://developer.hashicorp.com/terraform/install) >= 1.5.0
- [Foundry](https://book.getfoundry.sh/getting-started/installation) (forge, cast)
- [gcloud CLI](https://cloud.google.com/sdk/docs/install) authenticated
- GCP project with billing enabled
- Base Sepolia ETH in deployer wallet
- RPC endpoint (Alchemy, Infura, or public)

## Deployment Steps

### 1. Set Up Deployer Wallet

Import your deployer private key into Foundry's encrypted keystore:

```bash
cast wallet import deployer --interactive
```

Verify it was added:

```bash
cast wallet list
```

### 2. Deploy Contracts to Base Sepolia

```bash
cd contracts

ADMIN_ADDRESS="0xYOUR_WALLET_ADDRESS" \
SEQUENCER_ADDRESS="0xYOUR_WALLET_ADDRESS" \
forge script script/DeployLocalDevEnv.s.sol:DeployLocalDevEnv \
    --rpc-url https://sepolia.base.org \
    --account deployer \
    --etherscan-api-key $BASESCAN_API_KEY \
    --verify \
    --broadcast \
    -vvv
```

Record the deployed contract addresses from the output:

```
== Logs ==
  MockWETH deployed at: 0x...
  MockAttestationVerifier deployed at: 0x...
  TeeKeyManager deployed at: 0x...
  Bridge deployed at: 0x...
  PriceOracle deployed at: 0x...
```

### 3. Create Terraform Configuration

```bash
cd deploy/terraform/environments/staging

# Copy the Base Sepolia template
cp base-sepolia.tfvars.template terraform.tfvars
```

Edit `terraform.tfvars` and fill in:

| Variable | Value |
|----------|-------|
| `project_id` | Your GCP project ID |
| `gcs_bucket_name` | Unique bucket name (e.g., `myproject-synddb-staging`) |
| `tee_bootstrap.key_manager_address` | TeeKeyManager address from step 2 |
| `tee_bootstrap.rpc_url` | Your Base Sepolia RPC URL |
| `tee_bootstrap.attestation_audience` | Your staging domain |
| `bridge_contract_address` | Bridge address from step 2 |

### 4. Initialize Terraform

```bash
terraform init
```

### 5. Review the Plan

```bash
terraform plan
```

This shows what resources will be created:
- VPC network and firewall rules
- GCS bucket for batch storage
- Service accounts with minimal permissions
- Sequencer Confidential VM
- Validator Confidential VM (with custom price consistency rules)
- Proof service (Cloud Run)

**Note:** The template uses `price-oracle-validator` image which includes custom validation
rules for price consistency. For other use cases, change `validator_image` to use
`synddb-validator` instead.

### 6. Deploy Infrastructure

```bash
terraform apply
```

Type `yes` when prompted.

### 7. Verify Deployment

Get the outputs:

```bash
terraform output
```

Check sequencer health:

```bash
SEQUENCER_IP=$(terraform output -raw sequencer_external_ip)
curl http://$SEQUENCER_IP:8433/health
```

Check validator health:

```bash
VALIDATOR_IP=$(terraform output -raw validator_external_ip)
curl http://$VALIDATOR_IP:8080/health
```

### 8. Verify TEE Key Registration

The sequencer and validator automatically register their TEE keys on startup. Verify:

```bash
# Get sequencer's signer address
SIGNER=$(curl -s http://$SEQUENCER_IP:8433/status | jq -r '.signer_address')

# Check if registered on-chain
cast call \
    --rpc-url https://sepolia.base.org \
    $TEE_KEY_MANAGER_ADDRESS \
    "isKeyValid(address)(bool)" \
    $SIGNER
```

## Price Oracle (Confidential Space)

The price oracle runs inside Confidential Space alongside the sequencer and validator,
ensuring the entire data pipeline is TEE-protected.

The price oracle image is **built automatically by CI** and pushed to Artifact Registry
on every push to `main` or `example-app`. No manual image building required.

### Configuration

In `terraform.tfvars`, configure the price oracle:

```hcl
# Image is built by CI - use edge for latest, or pin to a specific sha/version
price_oracle_image = "us-central1-docker.pkg.dev/synddb-infra/synddb/price-oracle:edge"

price_oracle_contract_address = "0x..."  # From contract deployment

price_oracle_config = {
  coingecko_api_key     = ""  # Optional - free tier works without
  cmc_api_key           = ""  # Optional
  fetch_interval        = 60
  assets                = ["BTC", "ETH", "SOL"]
  chain_monitor_enabled = true
}
```

Then apply:

```bash
terraform apply
```

### Verify the Price Oracle

Check the VM is running:

```bash
gcloud compute instances describe synddb-staging-price-oracle \
    --zone us-central1-a \
    --format='get(status)'
```

View logs:

```bash
gcloud compute instances get-serial-port-output synddb-staging-price-oracle \
    --zone us-central1-a
```

### Verify Data Flow

Check the sequencer received changesets:

```bash
SEQUENCER_IP=$(terraform output -raw sequencer_internal_ip)
# From within GCP or via IAP tunnel:
curl -s http://$SEQUENCER_IP:8433/status | jq '.total_changesets'
```

Check the validator replicated the state:

```bash
VALIDATOR_IP=$(terraform output -raw validator_internal_ip)
curl -s http://$VALIDATOR_IP:8080/status | jq '.replicated_sequence'
```

### End-to-End Test: On-Chain Price Request

Request a price update on-chain:

```bash
cast send \
    --rpc-url https://sepolia.base.org \
    --account deployer \
    $PRICE_ORACLE_CONTRACT_ADDRESS \
    "requestPrice(string)" \
    "BTC"
```

The price oracle (running in Confidential Space) will:
1. Detect the `PriceRequested` event via chain monitor
2. Fetch the price from CoinGecko/CoinMarketCap
3. Submit the changeset to the sequencer
4. The validator picks it up and submits to the bridge

## Teardown

To destroy all staging resources:

```bash
terraform destroy
```

Type `yes` when prompted. The GCS bucket will be deleted (force_destroy is enabled for staging).

## Troubleshooting

### View VM Logs

```bash
# Sequencer logs
gcloud compute instances get-serial-port-output synddb-staging-sequencer \
    --zone us-central1-a

# Validator logs
gcloud compute instances get-serial-port-output synddb-staging-validator \
    --zone us-central1-a
```

### SSH into VMs

```bash
# Sequencer
gcloud compute ssh synddb-staging-sequencer --zone us-central1-a

# Validator
gcloud compute ssh synddb-staging-validator --zone us-central1-a
```

### Check Container Status

```bash
# On the VM
sudo docker ps
sudo docker logs <container_id>
```

### TEE Key Registration Failed

If the sequencer/validator failed to register their TEE key:

1. Check the proof service logs in Cloud Run
2. Verify the `attestation_audience` matches your domain
3. Ensure the RPC URL is accessible from GCP

## Files

| File | Purpose |
|------|---------|
| `main.tf` | Infrastructure definition |
| `variables.tf` | Variable declarations |
| `outputs.tf` | Output values |
| `terraform.tfvars.example` | Generic template |
| `base-sepolia.tfvars.template` | Base Sepolia template |
| `terraform.tfvars` | Your configuration (gitignored) |

## Related Documentation

- [Contract Deployment](../../../../contracts/README.md)
- [Price Oracle Example](../../../../examples/price-oracle/README.md)
- [Reproducible Builds](../../../../docker/reproducible/README.md)
