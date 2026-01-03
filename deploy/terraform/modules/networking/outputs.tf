output "network_id" {
  description = "ID of the VPC network"
  value       = google_compute_network.synddb.id
}

output "network_self_link" {
  description = "Self-link of the VPC network"
  value       = google_compute_network.synddb.self_link
}

output "network_name" {
  description = "Name of the VPC network"
  value       = google_compute_network.synddb.name
}

output "subnet_id" {
  description = "ID of the subnet"
  value       = google_compute_subnetwork.synddb.id
}

output "subnet_self_link" {
  description = "Self-link of the subnet"
  value       = google_compute_subnetwork.synddb.self_link
}

output "subnet_name" {
  description = "Name of the subnet"
  value       = google_compute_subnetwork.synddb.name
}
