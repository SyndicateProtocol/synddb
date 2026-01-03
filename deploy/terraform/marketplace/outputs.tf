# =============================================================================
# Deployment Summary
# =============================================================================

output "deployment_name" {
  description = "Name of this deployment"
  value       = var.goog_cm_deployment_name
}

output "project_id" {
  description = "GCP project where resources are deployed"
  value       = var.project_id
}

output "region" {
  description = "GCP region for this deployment"
  value       = var.region
}

output "zone" {
  description = "GCP zone for compute instances"
  value       = var.zone
}

# =============================================================================
# Sequencer
# =============================================================================

output "sequencer_instance_name" {
  description = "Name of the sequencer VM instance"
  value       = google_compute_instance.sequencer.name
}

output "sequencer_internal_ip" {
  description = "Internal IP of the sequencer (use for validator connection)"
  value       = google_compute_instance.sequencer.network_interface[0].network_ip
}

output "sequencer_status_url" {
  description = "URL to check sequencer status (accessible from within VPC)"
  value       = "http://${google_compute_instance.sequencer.network_interface[0].network_ip}:8433/status"
}

output "sequencer_self_link" {
  description = "Self-link to the sequencer instance"
  value       = google_compute_instance.sequencer.self_link
}

# =============================================================================
# Validator
# =============================================================================

output "validator_instance_name" {
  description = "Name of the validator VM instance"
  value       = google_compute_instance.validator.name
}

output "validator_internal_ip" {
  description = "Internal IP of the validator"
  value       = google_compute_instance.validator.network_interface[0].network_ip
}

output "validator_status_url" {
  description = "URL to check validator status (accessible from within VPC)"
  value       = "http://${google_compute_instance.validator.network_interface[0].network_ip}:8080/status"
}

output "validator_self_link" {
  description = "Self-link to the validator instance"
  value       = google_compute_instance.validator.self_link
}

# =============================================================================
# Storage
# =============================================================================

output "gcs_bucket_name" {
  description = "GCS bucket name for batch storage"
  value       = google_storage_bucket.synddb.name
}

output "gcs_bucket_url" {
  description = "GCS bucket URL"
  value       = google_storage_bucket.synddb.url
}

# =============================================================================
# Proof Service (Optional)
# =============================================================================

output "proof_service_url" {
  description = "URL of the proof service (if deployed)"
  value       = var.deploy_proof_service ? google_cloud_run_v2_service.proof_service[0].uri : null
}

output "proof_service_name" {
  description = "Name of the proof service (if deployed)"
  value       = var.deploy_proof_service ? google_cloud_run_v2_service.proof_service[0].name : null
}

# =============================================================================
# Networking
# =============================================================================

output "network_name" {
  description = "Name of the VPC network"
  value       = google_compute_network.synddb.name
}

output "subnet_name" {
  description = "Name of the subnet"
  value       = google_compute_subnetwork.synddb.name
}

# =============================================================================
# GCP Console Links
# =============================================================================

output "sequencer_console_url" {
  description = "Link to sequencer VM in GCP Console"
  value       = "https://console.cloud.google.com/compute/instancesDetail/zones/${var.zone}/instances/${google_compute_instance.sequencer.name}?project=${var.project_id}"
}

output "validator_console_url" {
  description = "Link to validator VM in GCP Console"
  value       = "https://console.cloud.google.com/compute/instancesDetail/zones/${var.zone}/instances/${google_compute_instance.validator.name}?project=${var.project_id}"
}

output "logs_console_url" {
  description = "Link to Cloud Logging for this deployment"
  value       = "https://console.cloud.google.com/logs/query;query=resource.labels.instance_id%3D%22${google_compute_instance.sequencer.instance_id}%22%20OR%20resource.labels.instance_id%3D%22${google_compute_instance.validator.instance_id}%22?project=${var.project_id}"
}

output "bucket_console_url" {
  description = "Link to GCS bucket in GCP Console"
  value       = "https://console.cloud.google.com/storage/browser/${google_storage_bucket.synddb.name}?project=${var.project_id}"
}

# =============================================================================
# Verification Commands
# =============================================================================

output "verification_commands" {
  description = "Commands to verify the deployment is working"
  value       = <<-EOT
    # View sequencer logs:
    gcloud compute instances get-serial-port-output ${google_compute_instance.sequencer.name} --zone=${var.zone} --project=${var.project_id}

    # View validator logs:
    gcloud compute instances get-serial-port-output ${google_compute_instance.validator.name} --zone=${var.zone} --project=${var.project_id}

    # Check sequencer status (from a VM in the same VPC):
    curl http://${google_compute_instance.sequencer.network_interface[0].network_ip}:8433/status

    # Check validator status (from a VM in the same VPC):
    curl http://${google_compute_instance.validator.network_interface[0].network_ip}:8080/status

    # View Cloud Logging:
    gcloud logging read 'resource.labels.instance_id="${google_compute_instance.sequencer.instance_id}" OR resource.labels.instance_id="${google_compute_instance.validator.instance_id}"' --project=${var.project_id} --limit=50
  EOT
}
