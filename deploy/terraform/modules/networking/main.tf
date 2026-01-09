terraform {
  required_providers {
    google = {
      source  = "hashicorp/google"
      version = ">= 5.0.0"
    }
  }
}

# VPC Network
resource "google_compute_network" "synddb" {
  name                    = "${var.name_prefix}-network"
  project                 = var.project_id
  auto_create_subnetworks = false
  routing_mode            = "REGIONAL"
}

# Subnet
resource "google_compute_subnetwork" "synddb" {
  name                     = "${var.name_prefix}-subnet"
  project                  = var.project_id
  region                   = var.region
  network                  = google_compute_network.synddb.id
  ip_cidr_range            = var.subnet_cidr
  private_ip_google_access = var.enable_private_google_access
}

# Firewall: Allow internal communication
resource "google_compute_firewall" "internal" {
  name    = "${var.name_prefix}-allow-internal"
  project = var.project_id
  network = google_compute_network.synddb.id

  allow {
    protocol = "tcp"
    ports    = ["0-65535"]
  }

  allow {
    protocol = "udp"
    ports    = ["0-65535"]
  }

  allow {
    protocol = "icmp"
  }

  source_ranges = [var.subnet_cidr]
}

# Firewall: Allow health checks from GCP load balancers
resource "google_compute_firewall" "health_checks" {
  name    = "${var.name_prefix}-allow-health-checks"
  project = var.project_id
  network = google_compute_network.synddb.id

  allow {
    protocol = "tcp"
    ports    = ["8080", "8433"]
  }

  # GCP health check IP ranges
  source_ranges = ["35.191.0.0/16", "130.211.0.0/22"]
  target_tags   = ["${var.name_prefix}-sequencer", "${var.name_prefix}-validator"]
}

# Firewall: Allow SSH for debug instances (optional)
resource "google_compute_firewall" "ssh" {
  count   = length(var.allowed_ssh_ranges) > 0 ? 1 : 0
  name    = "${var.name_prefix}-allow-ssh"
  project = var.project_id
  network = google_compute_network.synddb.id

  allow {
    protocol = "tcp"
    ports    = ["22"]
  }

  source_ranges = var.allowed_ssh_ranges
  target_tags   = ["${var.name_prefix}-debug"]
}

# Cloud NAT for outbound internet access (required for container pulls)
resource "google_compute_router" "synddb" {
  name    = "${var.name_prefix}-router"
  project = var.project_id
  region  = var.region
  network = google_compute_network.synddb.id
}

resource "google_compute_router_nat" "synddb" {
  name                               = "${var.name_prefix}-nat"
  project                            = var.project_id
  router                             = google_compute_router.synddb.name
  region                             = var.region
  nat_ip_allocate_option             = "AUTO_ONLY"
  source_subnetwork_ip_ranges_to_nat = "ALL_SUBNETWORKS_ALL_IP_RANGES"

  log_config {
    enable = true
    filter = "ERRORS_ONLY"
  }
}

# Static internal IPs for services
# These persist across VM recreation, ensuring dependent services always have a stable address.
# Note: Currently supports single instances only. For multiple instances (e.g., multiple
# validators), add count/for_each here and update the service modules accordingly.

resource "google_compute_address" "sequencer" {
  name         = "${var.name_prefix}-sequencer-ip"
  project      = var.project_id
  region       = var.region
  address_type = "INTERNAL"
  subnetwork   = google_compute_subnetwork.synddb.id
  purpose      = "GCE_ENDPOINT"
}

resource "google_compute_address" "validator" {
  name         = "${var.name_prefix}-validator-ip"
  project      = var.project_id
  region       = var.region
  address_type = "INTERNAL"
  subnetwork   = google_compute_subnetwork.synddb.id
  purpose      = "GCE_ENDPOINT"
}

resource "google_compute_address" "price_oracle" {
  name         = "${var.name_prefix}-price-oracle-ip"
  project      = var.project_id
  region       = var.region
  address_type = "INTERNAL"
  subnetwork   = google_compute_subnetwork.synddb.id
  purpose      = "GCE_ENDPOINT"
}
