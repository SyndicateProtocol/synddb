terraform {
  required_providers {
    google = {
      source  = "hashicorp/google"
      version = ">= 5.0.0"
    }
  }
}

# Cloud Run v2 service for SP1 Network Prover (offloads proving to Succinct)
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
        container_port = 8080
      }

      resources {
        limits = {
          cpu    = var.cpu_limit
          memory = var.memory_limit
        }
        cpu_idle          = false
        startup_cpu_boost = true
      }

      env {
        name  = "SP1_PROVER"
        value = "network"
      }

      env {
        name  = "NETWORK_PRIVATE_KEY"
        value = var.sp1_network_private_key
      }

      env {
        name  = "LOG_JSON"
        value = "true"
      }

      env {
        name  = "RUST_LOG"
        value = "info"
      }

      startup_probe {
        http_get {
          path = "/health"
          port = 8080
        }
        initial_delay_seconds = 10
        period_seconds        = 10
        failure_threshold     = 30
        timeout_seconds       = 5
      }

      liveness_probe {
        http_get {
          path = "/health"
          port = 8080
        }
        period_seconds    = 30
        timeout_seconds   = 5
        failure_threshold = 3
      }
    }

    annotations = {
      "run.googleapis.com/startup-cpu-boost" = "true"
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
