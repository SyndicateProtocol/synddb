output "instance_id" {
  description = "Validator instance ID"
  value       = module.confidential_vm.instance_id
}

output "instance_name" {
  description = "Validator instance name"
  value       = module.confidential_vm.instance_name
}

output "internal_ip" {
  description = "Validator internal IP"
  value       = module.confidential_vm.internal_ip
}

output "external_ip" {
  description = "Validator external IP (if assigned)"
  value       = module.confidential_vm.external_ip
}

output "status_url" {
  description = "URL to check validator status (internal)"
  value       = "http://${module.confidential_vm.internal_ip}:8080/status"
}

output "health_url" {
  description = "URL for health checks (internal)"
  value       = "http://${module.confidential_vm.internal_ip}:8080/health"
}
