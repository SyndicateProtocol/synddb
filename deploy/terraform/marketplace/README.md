# SyndDB GCP Marketplace Package

Self-contained Terraform package for GCP Marketplace deployment.

## Overview

This package deploys SyndDB with:
- **Sequencer**: Confidential Space VM for message ordering and signing
- **Validator**: Confidential Space VM for state validation and sync
- **Proof Service** (optional): Cloud Run for SP1 attestation proofs
- **Storage**: GCS bucket for batch storage
- **Networking**: Private VPC with NAT for outbound access

## Prerequisites

1. GCP project with billing enabled
2. Required APIs enabled (automated by this package)
3. Container images in Artifact Registry

## Deployment

### Via Marketplace UI

1. Navigate to SyndDB in GCP Marketplace
2. Click "Configure"
3. Fill in required parameters
4. Review and deploy

### Via Terraform CLI

```bash
# Initialize
terraform init

# Create terraform.tfvars with your configuration
cp terraform.tfvars.example terraform.tfvars
# Edit terraform.tfvars

# Review plan
terraform plan

# Deploy
terraform apply
```

## Configuration

| Variable | Required | Description |
|----------|----------|-------------|
| `project_id` | Yes | GCP project ID |
| `goog_cm_deployment_name` | Yes | Deployment name (auto-provided by Marketplace) |
| `region` | No | GCP region (default: us-central1) |
| `zone` | No | GCP zone (default: us-central1-a) |
| `sequencer_image` | Yes | Sequencer container image URI |
| `validator_image` | Yes | Validator container image URI |
| `gcs_bucket_name` | Yes | GCS bucket name for batch storage |
| `enable_key_bootstrap` | No | Enable TEE key bootstrapping (default: false) |
| `deploy_proof_service` | No | Deploy proof service (default: false) |

### TEE Bootstrap Configuration

When `enable_key_bootstrap = true`:

| Variable | Required | Description |
|----------|----------|-------------|
| `tee_key_manager_address` | Yes | TeeKeyManager contract address |
| `bootstrap_rpc_url` | Yes | Ethereum RPC endpoint |
| `bootstrap_chain_id` | Yes | Chain ID for transactions |
| `attestation_audience` | Yes | Expected audience for attestation |
| `proof_service_image` | Yes | Proof service container image |

## Outputs

| Output | Description |
|--------|-------------|
| `sequencer_internal_ip` | Internal IP of sequencer VM |
| `validator_internal_ip` | Internal IP of validator VM |
| `gcs_bucket_url` | GCS bucket URL |
| `proof_service_url` | Proof service URL (if deployed) |
| `network_name` | VPC network name |

## Architecture

```
┌─────────────────────────────────────────────────────┐
│                    VPC Network                       │
│  ┌───────────────┐       ┌───────────────┐          │
│  │   Sequencer   │       │   Validator   │          │
│  │  (CS VM, TEE) │◄─────►│  (CS VM, TEE) │          │
│  │   :8433       │       │   :8080       │          │
│  └───────┬───────┘       └───────┬───────┘          │
│          │                       │                   │
│          ▼                       ▼                   │
│  ┌───────────────────────────────────────┐          │
│  │           GCS Bucket                  │          │
│  │   (batches/*.cbor.zst)                │          │
│  └───────────────────────────────────────┘          │
│                                                      │
│  ┌───────────────┐                                  │
│  │ Proof Service │ (Cloud Run, optional)            │
│  │   :8080       │                                  │
│  └───────────────┘                                  │
└─────────────────────────────────────────────────────┘
                         │
                    Cloud NAT
                         │
                    Internet
```

## Security

- VMs run in GCP Confidential Space (AMD SEV)
- No external IPs on VMs (egress via Cloud NAT)
- Uniform bucket-level access on GCS
- Minimal IAM permissions per service
- Internal-only ingress for proof service

## Support

- GitHub: https://github.com/SyndicateProtocol/synddb
- Documentation: https://github.com/SyndicateProtocol/synddb/blob/main/PLAN_DEPLOYMENT.md
