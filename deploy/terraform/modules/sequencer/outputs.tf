output "instance_id" {
  description = "Sequencer instance ID"
  value       = module.confidential_vm.instance_id
}

output "instance_name" {
  description = "Sequencer instance name"
  value       = module.confidential_vm.instance_name
}

output "internal_ip" {
  description = "Sequencer internal IP"
  value       = module.confidential_vm.internal_ip
}

output "external_ip" {
  description = "Sequencer external IP (if assigned)"
  value       = module.confidential_vm.external_ip
}

output "status_url" {
  description = "URL to check sequencer status (internal)"
  value       = "http://${module.confidential_vm.internal_ip}:8433/status"
}

output "health_url" {
  description = "URL for health checks (internal)"
  value       = "http://${module.confidential_vm.internal_ip}:8433/health"
}
