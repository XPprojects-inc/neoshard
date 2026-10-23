# NeoShard on Yandex Cloud (ru-central1): one VM running docker compose,
# Yandex Managed PostgreSQL, Object Storage for files. All data stays in the RF region (152-FZ).
#   export YC_TOKEN=... YC_CLOUD_ID=... YC_FOLDER_ID=...
#   terraform init && terraform apply -var domain=chat.example.ru -var ssh_public_key="$(cat ~/.ssh/id_ed25519.pub)"

terraform {
  required_version = ">= 1.5"
  required_providers {
    yandex = { source = "yandex-cloud/yandex", version = "~> 0.130" }
    random = { source = "hashicorp/random", version = "~> 3.6" }
  }
}

provider "yandex" {
  zone = var.zone
}

resource "yandex_vpc_network" "net" {
  name = "neoshard"
}

resource "yandex_vpc_subnet" "subnet" {
  name           = "neoshard-a"
  zone           = var.zone
  network_id     = yandex_vpc_network.net.id
  v4_cidr_blocks = ["10.10.0.0/24"]
}

resource "yandex_vpc_address" "ip" {
  name = "neoshard-ip"
  external_ipv4_address { zone_id = var.zone }
}

resource "yandex_vpc_security_group" "sg" {
  name       = "neoshard"
  network_id = yandex_vpc_network.net.id
  ingress {
    protocol       = "TCP"
    port           = 22
    v4_cidr_blocks = var.admin_cidrs
  }
  ingress {
    protocol       = "TCP"
    from_port      = 80
    to_port        = 443
    v4_cidr_blocks = ["0.0.0.0/0"]
  }
  ingress {
    protocol       = "UDP"
    port           = 443
    v4_cidr_blocks = ["0.0.0.0/0"]
  }
  ingress {
    protocol       = "TCP"
    from_port      = 7880
    to_port        = 7881
    v4_cidr_blocks = ["0.0.0.0/0"]
  }
  ingress {
    protocol       = "UDP"
    from_port      = 7882
    to_port        = 7882
    v4_cidr_blocks = ["0.0.0.0/0"]
  }
  ingress {
    protocol       = "UDP"
    from_port      = 50000
    to_port        = 50100
    v4_cidr_blocks = ["0.0.0.0/0"]
  }
  ingress {
    protocol       = "ANY"
    from_port      = 3478
    to_port        = 3478
    v4_cidr_blocks = ["0.0.0.0/0"]
  }
  ingress {
    protocol       = "TCP"
    port           = 5349
    v4_cidr_blocks = ["0.0.0.0/0"]
  }
  egress {
    protocol       = "ANY"
    v4_cidr_blocks = ["0.0.0.0/0"]
  }
}

# ---------------------------------------------------------------- database
resource "random_password" "db" {
  length  = 32
  special = false
}

resource "yandex_mdb_postgresql_cluster" "pg" {
  name        = "neoshard"
  environment = "PRODUCTION"
  network_id  = yandex_vpc_network.net.id
  config {
    version = "16"
    resources {
      resource_preset_id = var.db_preset
      disk_type_id       = "network-ssd"
      disk_size          = var.db_disk_gb
    }
    backup_retain_period_days = 14
  }
  host {
    zone      = var.zone
    subnet_id = yandex_vpc_subnet.subnet.id
  }
}

resource "yandex_mdb_postgresql_user" "neoshard" {
  cluster_id = yandex_mdb_postgresql_cluster.pg.id
  name       = "neoshard"
  password   = random_password.db.result
}

resource "yandex_mdb_postgresql_database" "neoshard" {
  cluster_id = yandex_mdb_postgresql_cluster.pg.id
  name       = "neoshard"
  owner      = yandex_mdb_postgresql_user.neoshard.name
}

# ---------------------------------------------------------------- object storage
resource "yandex_iam_service_account" "s3" {
  name = "neoshard-s3"
}

resource "yandex_resourcemanager_folder_iam_member" "s3_editor" {
  folder_id = var.folder_id
  role      = "storage.editor"
  member    = "serviceAccount:${yandex_iam_service_account.s3.id}"
}

resource "yandex_iam_service_account_static_access_key" "s3" {
  service_account_id = yandex_iam_service_account.s3.id
}

resource "yandex_storage_bucket" "files" {
  bucket     = var.bucket
  access_key = yandex_iam_service_account_static_access_key.s3.access_key
  secret_key = yandex_iam_service_account_static_access_key.s3.secret_key
  anonymous_access_flags {
    read = true # attachments are public by unguessable key; use a CDN in front
    list = false
  }
  cors_rule {
    allowed_methods = ["PUT", "GET"]
    allowed_origins = ["https://${var.domain}"]
    allowed_headers = ["*"]
  }
}

# ---------------------------------------------------------------- application VM
resource "random_password" "secret_key" {
  length = 32
}

resource "random_password" "livekit" {
  length  = 40
  special = false
}

resource "random_password" "turn" {
  length  = 40
  special = false
}

data "yandex_compute_image" "ubuntu" {
  family = "ubuntu-2404-lts"
}

resource "yandex_compute_instance" "app" {
  name        = "neoshard-app"
  platform_id = "standard-v3"
  resources {
    cores  = var.vm_cores
    memory = var.vm_memory_gb
  }
  boot_disk {
    initialize_params {
      image_id = data.yandex_compute_image.ubuntu.id
      size     = 40
      type     = "network-ssd"
    }
  }
  network_interface {
    subnet_id          = yandex_vpc_subnet.subnet.id
    nat                = true
    nat_ip_address     = yandex_vpc_address.ip.external_ipv4_address[0].address
    security_group_ids = [yandex_vpc_security_group.sg.id]
  }
  metadata = {
    ssh-keys = "ubuntu:${var.ssh_public_key}"
    user-data = templatefile("${path.module}/cloud-init.yaml", {
      domain       = var.domain
      repo         = var.repo
      db_url       = "postgres://neoshard:${random_password.db.result}@${yandex_mdb_postgresql_cluster.pg.host[0].fqdn}:6432/neoshard?sslmode=require"
      secret_key   = base64encode(substr(random_password.secret_key.result, 0, 32))
      livekit_key  = random_password.livekit.result
      turn_secret  = random_password.turn.result
      s3_key       = yandex_iam_service_account_static_access_key.s3.access_key
      s3_secret    = yandex_iam_service_account_static_access_key.s3.secret_key
      bucket       = var.bucket
    })
  }
}
