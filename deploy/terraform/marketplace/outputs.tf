output "sequencer_internal_ip" {
  description = "Internal IP of the sequencer"
  value       = google_compute_instance.sequencer.network_interface[0].network_ip
}

output "validator_internal_ip" {
  description = "Internal IP of the validator"
  value       = google_compute_instance.validator.network_interface[0].network_ip
}

output "gcs_bucket_url" {
  description = "GCS bucket URL for batch storage"
  value       = google_storage_bucket.synddb.url
}

output "proof_service_url" {
  description = "URL of the proof service (if deployed)"
  value       = var.deploy_proof_service ? google_cloud_run_v2_service.proof_service[0].uri : null
}

output "network_name" {
  description = "Name of the VPC network"
  value       = google_compute_network.synddb.name
}

output "sequencer_status_url" {
  description = "URL to check sequencer status (internal only)"
  value       = "http://${google_compute_instance.sequencer.network_interface[0].network_ip}:8433/status"
}

output "validator_status_url" {
  description = "URL to check validator status (internal only)"
  value       = "http://${google_compute_instance.validator.network_interface[0].network_ip}:8080/status"
}
