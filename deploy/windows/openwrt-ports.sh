#!/bin/sh
# OpenWRT: a fixed LAN address for the NeoShard host and the port forwards it needs.
# Run on the router (by its owner), with the host's LAN address and MAC:
#   ssh root@<router> "HOST_IP=192.168.1.50 HOST_MAC=AA:BB:CC:DD:EE:FF sh -s" < openwrt-ports.sh
# Re-running is safe: every section has a fixed name.
: "${HOST_IP:?LAN address of the NeoShard host, e.g. 192.168.1.50}"
: "${HOST_MAC:?MAC address of the host network card}"

uci set dhcp.neoshard=host
uci set dhcp.neoshard.name='neoshard'
uci set dhcp.neoshard.mac="$HOST_MAC"
uci set dhcp.neoshard.ip="$HOST_IP"

fwd() { # name proto port(s)
	uci set firewall."$1"=redirect
	uci set firewall."$1".name="$1"
	uci set firewall."$1".target='DNAT'
	uci set firewall."$1".src='wan'
	uci set firewall."$1".dest='lan'
	uci set firewall."$1".proto="$2"
	uci set firewall."$1".src_dport="$3"
	uci set firewall."$1".dest_ip="$HOST_IP"
	uci set firewall."$1".dest_port="$3"
}
fwd neoshard_http tcp 80        # certificates (Let's Encrypt) and redirect to https
fwd neoshard_https tcp 443      # web client, API, gateway
fwd neoshard_lk_tcp tcp 7880-7881 # voice: signalling (TLS) and RTC over TCP
fwd neoshard_lk_udp udp 7882    # voice: RTC over UDP
fwd neoshard_lk_range udp 50000-50100

uci commit dhcp
uci commit firewall
/etc/init.d/firewall reload
/etc/init.d/dnsmasq reload
echo "NeoShard: lease $HOST_IP for $HOST_MAC, ports forwarded"
