variable "folder_id" { type = string }
variable "zone" {
  type    = string
  default = "ru-central1-a"
}
variable "domain" { type = string }
variable "ssh_public_key" { type = string }
variable "admin_cidrs" {
  type    = list(string)
  default = ["0.0.0.0/0"]
}
variable "repo" {
  type    = string
  default = "https://github.com/neoshard/neoshard.git"
}
variable "bucket" {
  type    = string
  default = "neoshard-files"
}
variable "vm_cores" {
  type    = number
  default = 4
}
variable "vm_memory_gb" {
  type    = number
  default = 8
}
variable "db_preset" {
  type    = string
  default = "s3-c2-m8"
}
variable "db_disk_gb" {
  type    = number
  default = 50
}
