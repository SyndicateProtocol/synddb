output "sequencer_internal_ip" {
  description = "Internal IP of the sequencer"
  value       = module.sequencer.internal_ip
}

output "sequencer_status_url" {
  description = "URL to check sequencer status"
  value       = module.sequencer.status_url
}

output "validator_internal_ip" {
  description = "Internal IP of the validator"
  value       = module.validator.internal_ip
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
  description = "URL of the proof service"
  value       = length(module.proof_service) > 0 ? module.proof_service[0].service_url : null
}

output "relayer_url" {
  description = "URL of the relayer service"
  value       = length(module.relayer) > 0 ? module.relayer[0].service_url : null
}

output "network_name" {
  description = "Name of the VPC network"
  value       = module.networking.network_name
}

output "price_oracle_internal_ip" {
  description = "Internal IP of the price oracle"
  value       = length(module.price_oracle) > 0 ? module.price_oracle[0].internal_ip : null
}

output "price_oracle_instance_name" {
  description = "Instance name of the price oracle"
  value       = length(module.price_oracle) > 0 ? module.price_oracle[0].instance_name : null
}

output "resolved_images" {
  description = "Resolved container images with digests (from tag or explicit digest)"
  value = var.tee_bootstrap != null ? {
    sequencer     = local.sequencer_resolved_image
    validator     = local.validator_resolved_image
    proof_service = local.proof_service_resolved_image
    price_oracle  = var.price_oracle_config != null ? local.price_oracle_resolved_image : null
  } : null
}
