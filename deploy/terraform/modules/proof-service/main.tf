terraform {
  required_providers {
    google = {
      source  = "hashicorp/google"
      version = ">= 5.0.0"
    }
  }
}

# Local variable to determine if GPU should be enabled
locals {
  use_gpu = var.prover_backend == "risc0" || var.enable_gpu
}

# Cloud Run v2 service for proof generation
# - SP1: Network prover (offloads proving to Succinct)
# - RISC Zero: GPU proving (local CUDA on Cloud Run L4 GPUs)
resource "google_cloud_run_v2_service" "proof_service" {
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
        container_port = 8083
      }

      resources {
        limits = merge(
          {
            cpu    = var.cpu_limit
            memory = var.memory_limit
          },
          # Add GPU resource limit for RISC Zero proving
          local.use_gpu ? { "nvidia.com/gpu" = tostring(var.gpu_count) } : {}
        )
        cpu_idle          = false
        startup_cpu_boost = true
      }

      # SP1-specific environment variables
      dynamic "env" {
        for_each = var.prover_backend == "sp1" ? [1] : []
        content {
          name  = "SP1_PROVER"
          value = "network"
        }
      }

      dynamic "env" {
        for_each = var.prover_backend == "sp1" ? [1] : []
        content {
          name  = "NETWORK_PRIVATE_KEY"
          value = var.sp1_network_private_key
        }
      }

      # Common environment variables
      env {
        name  = "LOG_JSON"
        value = "true"
      }

      env {
        name  = "RUST_LOG"
        value = var.prover_backend == "risc0" ? "info,risc0_zkvm=debug" : "info"
      }

      startup_probe {
        http_get {
          path = "/health"
          port = 8083
        }
        initial_delay_seconds = 10
        period_seconds        = 10
        failure_threshold     = 30
        timeout_seconds       = 5
      }

      liveness_probe {
        http_get {
          path = "/health"
          port = 8083
        }
        period_seconds    = 30
        timeout_seconds   = 5
        failure_threshold = 3
      }
    }

    # GPU node selector for RISC Zero proving
    dynamic "node_selector" {
      for_each = local.use_gpu ? [1] : []
      content {
        accelerator = var.gpu_type
      }
    }

    annotations = {
      "run.googleapis.com/startup-cpu-boost" = "true"
      # Force new revision when image changes - use digest as annotation value
      "synddb.io/image-digest" = regex("sha256:[a-f0-9]+", var.container_image)
    }

    max_instance_request_concurrency = var.concurrency
  }

  labels = var.labels

  # Allow deletion for staging environments
  deletion_protection = false

  lifecycle {
    ignore_changes = [
      # Ignore client-set annotations
      template[0].annotations["client.knative.dev/user-image"],
    ]
  }
}

# IAM policy for unauthenticated access (if enabled)
resource "google_cloud_run_v2_service_iam_member" "allow_unauthenticated" {
  count    = var.allow_unauthenticated ? 1 : 0
  project  = var.project_id
  location = var.region
  name     = google_cloud_run_v2_service.proof_service.name
  role     = "roles/run.invoker"
  member   = "allUsers"
}

# IAM policy for service accounts that can invoke the proof service
resource "google_cloud_run_v2_service_iam_member" "invoker" {
  for_each = toset(var.invoker_service_accounts)
  project  = var.project_id
  location = var.region
  name     = google_cloud_run_v2_service.proof_service.name
  role     = "roles/run.invoker"
  member   = "serviceAccount:${each.value}"
}
