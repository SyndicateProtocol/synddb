output "bucket_name" {
  description = "Name of the created bucket"
  value       = google_storage_bucket.synddb.name
}

output "bucket_url" {
  description = "URL of the bucket (gs://...)"
  value       = google_storage_bucket.synddb.url
}

output "bucket_self_link" {
  description = "Self-link of the bucket"
  value       = google_storage_bucket.synddb.self_link
}

output "bucket_id" {
  description = "ID of the bucket"
  value       = google_storage_bucket.synddb.id
}
