output "instance_id" {
  description = "Instance ID"
  value       = google_compute_instance.confidential_vm.instance_id
}

output "instance_self_link" {
  description = "Self-link of the instance"
  value       = google_compute_instance.confidential_vm.self_link
}

output "instance_name" {
  description = "Name of the instance"
  value       = google_compute_instance.confidential_vm.name
}

output "internal_ip" {
  description = "Internal IP address"
  value       = google_compute_instance.confidential_vm.network_interface[0].network_ip
}

output "external_ip" {
  description = "External IP address (if assigned)"
  value       = try(google_compute_instance.confidential_vm.network_interface[0].access_config[0].nat_ip, null)
}

output "zone" {
  description = "Zone where the instance is deployed"
  value       = google_compute_instance.confidential_vm.zone
}
