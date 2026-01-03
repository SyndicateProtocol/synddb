output "sequencer_internal_ip" {
  description = "Internal IP of the sequencer"
  value       = module.sequencer.internal_ip
}

output "sequencer_status_url" {
  description = "URL to check sequencer status"
  value       = module.sequencer.status_url
}

output "validator_internal_ips" {
  description = "Internal IPs of validators"
  value       = [for v in module.validator : v.internal_ip]
}

output "validator_status_urls" {
  description = "URLs to check validator status"
  value       = [for v in module.validator : v.status_url]
}

output "gcs_bucket_url" {
  description = "GCS bucket URL for batch storage"
  value       = module.storage.bucket_url
}

output "proof_service_url" {
  description = "URL of the proof service"
  value       = module.proof_service.service_url
}

output "network_name" {
  description = "Name of the VPC network"
  value       = module.networking.network_name
}
