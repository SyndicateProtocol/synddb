output "sequencer_service_account_email" {
  description = "Email of the sequencer service account"
  value       = google_service_account.sequencer.email
}

output "sequencer_service_account_id" {
  description = "ID of the sequencer service account"
  value       = google_service_account.sequencer.id
}

output "validator_service_account_email" {
  description = "Email of the validator service account"
  value       = google_service_account.validator.email
}

output "validator_service_account_id" {
  description = "ID of the validator service account"
  value       = google_service_account.validator.id
}

output "proof_service_account_email" {
  description = "Email of the proof service account"
  value       = google_service_account.proof_service.email
}

output "proof_service_account_id" {
  description = "ID of the proof service account"
  value       = google_service_account.proof_service.id
}

output "relayer_service_account_email" {
  description = "Email of the relayer service account"
  value       = google_service_account.relayer.email
}

output "relayer_service_account_id" {
  description = "ID of the relayer service account"
  value       = google_service_account.relayer.id
}

output "price_oracle_service_account_email" {
  description = "Email of the price oracle service account"
  value       = google_service_account.price_oracle.email
}

output "price_oracle_service_account_id" {
  description = "ID of the price oracle service account"
  value       = google_service_account.price_oracle.id
}
