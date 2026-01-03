output "sequencer_internal_ip" {
  description = "Internal IP of the sequencer"
  value       = module.sequencer.internal_ip
}

output "sequencer_external_ip" {
  description = "External IP of the sequencer (if debug mode)"
  value       = module.sequencer.external_ip
}

output "sequencer_status_url" {
  description = "URL to check sequencer status"
  value       = module.sequencer.status_url
}

output "validator_internal_ip" {
  description = "Internal IP of the validator"
  value       = module.validator.internal_ip
}

output "validator_external_ip" {
  description = "External IP of the validator (if debug mode)"
  value       = module.validator.external_ip
}

output "validator_status_url" {
  description = "URL to check validator status"
  value       = module.validator.status_url
}

output "gcs_bucket_url" {
  description = "GCS bucket URL for batch storage"
  value       = module.storage.bucket_url
}

output "proof_service_url" {
  description = "URL of the proof service (if deployed)"
  value       = var.deploy_proof_service ? module.proof_service[0].service_url : null
}

output "network_name" {
  description = "Name of the VPC network"
  value       = module.networking.network_name
}
