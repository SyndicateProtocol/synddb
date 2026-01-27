output "instance_name" {
  description = "Name of the price oracle instance"
  value       = module.confidential_vm.instance_name
}

output "instance_id" {
  description = "Instance ID"
  value       = module.confidential_vm.instance_id
}

output "internal_ip" {
  description = "Internal IP address"
  value       = module.confidential_vm.internal_ip
}

output "external_ip" {
  description = "External IP address (if enabled)"
  value       = module.confidential_vm.external_ip
}
