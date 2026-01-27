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
- Validator Confidential VM
- Proof service (Cloud Run)

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
