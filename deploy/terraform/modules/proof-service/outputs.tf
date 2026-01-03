output "service_url" {
  description = "Cloud Run service URL"
  value       = google_cloud_run_v2_service.proof_service.uri
}

output "service_name" {
  description = "Cloud Run service name"
  value       = google_cloud_run_v2_service.proof_service.name
}

output "service_id" {
  description = "Cloud Run service ID"
  value       = google_cloud_run_v2_service.proof_service.id
}

output "latest_revision" {
  description = "Latest ready revision"
  value       = google_cloud_run_v2_service.proof_service.latest_ready_revision
}
