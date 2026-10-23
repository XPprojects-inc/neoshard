output "public_ip" { value = yandex_vpc_address.ip.external_ipv4_address[0].address }
output "next_step" { value = "Point ${var.domain} (A record) to the IP above; Caddy obtains TLS automatically." }
