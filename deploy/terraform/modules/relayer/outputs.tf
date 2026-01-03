output "service_url" {
  description = "Cloud Run service URL"
  value       = google_cloud_run_v2_service.relayer.uri
}

output "service_name" {
  description = "Cloud Run service name"
  value       = google_cloud_run_v2_service.relayer.name
}

output "service_id" {
  description = "Cloud Run service ID"
  value       = google_cloud_run_v2_service.relayer.id
}

output "latest_revision" {
  description = "Latest ready revision"
  value       = google_cloud_run_v2_service.relayer.latest_ready_revision
}

output "secret_id" {
  description = "Secret Manager secret ID for private key"
  value       = google_secret_manager_secret.relayer_private_key.secret_id
}
