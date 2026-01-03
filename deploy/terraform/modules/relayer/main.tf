# SyndDB Gas Relayer Service
#
# Cloud Run service that handles TEE key registration and gas funding.
# This is a standard service (no TEE, no GPU required).

terraform {
  required_providers {
    google = {
      source  = "hashicorp/google"
      version = ">= 5.0.0"
    }
  }
}

# Secret for relayer private key
resource "google_secret_manager_secret" "relayer_private_key" {
  project   = var.project_id
  secret_id = "${var.service_name}-private-key"

  replication {
    auto {}
  }

  labels = var.labels
}

# Initial secret version (placeholder - should be updated manually or via CI)
resource "google_secret_manager_secret_version" "relayer_private_key" {
  count       = var.private_key != "" ? 1 : 0
  secret      = google_secret_manager_secret.relayer_private_key.id
  secret_data = var.private_key
}

# Grant service account access to the secret
resource "google_secret_manager_secret_iam_member" "relayer_secret_access" {
  project   = var.project_id
  secret_id = google_secret_manager_secret.relayer_private_key.secret_id
  role      = "roles/secretmanager.secretAccessor"
  member    = "serviceAccount:${var.service_account_email}"
}

# Cloud Run service
resource "google_cloud_run_v2_service" "relayer" {
  name     = var.service_name
  location = var.region
  project  = var.project_id
  ingress  = var.ingress == "all" ? "INGRESS_TRAFFIC_ALL" : var.ingress == "internal" ? "INGRESS_TRAFFIC_INTERNAL_ONLY" : "INGRESS_TRAFFIC_INTERNAL_LOAD_BALANCER"

  template {
    scaling {
      min_instance_count = var.min_instances
      max_instance_count = var.max_instances
    }

    timeout = "${var.timeout_seconds}s"

    service_account = var.service_account_email

    # VPC connector for private networking (optional)
    dynamic "vpc_access" {
      for_each = var.vpc_connector != "" ? [1] : []
      content {
        connector = var.vpc_connector
        egress    = var.vpc_egress == "all-traffic" ? "ALL_TRAFFIC" : "PRIVATE_RANGES_ONLY"
      }
    }

    containers {
      image = var.container_image

      ports {
        container_port = 8082
      }

      resources {
        limits = {
          cpu    = var.cpu_limit
          memory = var.memory_limit
        }
        cpu_idle          = true  # Allow CPU throttling when idle
        startup_cpu_boost = true
      }

      # Required environment variables
      env {
        name  = "RELAYER_LISTEN_ADDR"
        value = "0.0.0.0:8082"
      }

      env {
        name  = "RPC_URL"
        value = var.rpc_url
      }

      env {
        name  = "CHAIN_ID"
        value = tostring(var.chain_id)
      }

      env {
        name  = "TEE_KEY_MANAGER_CONTRACT_ADDRESS"
        value = var.key_manager_address
      }

      env {
        name  = "GAS_TREASURY_CONTRACT_ADDRESS"
        value = var.treasury_address
      }

      env {
        name  = "REQUIRED_AUDIENCE_HASH"
        value = var.required_audience_hash
      }

      env {
        name  = "ALLOWED_IMAGE_DIGESTS"
        value = join(",", var.allowed_image_digests)
      }

      env {
        name  = "MAX_FUNDING_PER_DIGEST_DAILY"
        value = var.max_funding_per_digest_daily
      }

      env {
        name  = "MAX_FUNDING_PER_ADDRESS"
        value = var.max_funding_per_address
      }

      # Private key from Secret Manager
      env {
        name = "RELAYER_PRIVATE_KEY"
        value_source {
          secret_key_ref {
            secret  = google_secret_manager_secret.relayer_private_key.secret_id
            version = "latest"
          }
        }
      }

      env {
        name  = "LOG_JSON"
        value = "true"
      }

      env {
        name  = "RUST_LOG"
        value = var.rust_log
      }

      startup_probe {
        http_get {
          path = "/health"
          port = 8082
        }
        initial_delay_seconds = 5
        period_seconds        = 5
        failure_threshold     = 12
        timeout_seconds       = 3
      }

      liveness_probe {
        http_get {
          path = "/health"
          port = 8082
        }
        period_seconds    = 30
        timeout_seconds   = 3
        failure_threshold = 3
      }
    }

    max_instance_request_concurrency = var.concurrency
  }

  labels = var.labels

  lifecycle {
    ignore_changes = [
      template[0].annotations["client.knative.dev/user-image"],
    ]
  }

  depends_on = [google_secret_manager_secret_iam_member.relayer_secret_access]
}

# IAM policy for unauthenticated access (if enabled)
resource "google_cloud_run_v2_service_iam_member" "allow_unauthenticated" {
  count    = var.allow_unauthenticated ? 1 : 0
  project  = var.project_id
  location = var.region
  name     = google_cloud_run_v2_service.relayer.name
  role     = "roles/run.invoker"
  member   = "allUsers"
}
