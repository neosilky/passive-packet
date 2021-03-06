#![feature(ip)]

#[macro_use]
extern crate serde_derive;
extern crate serde;
extern crate serde_json;
extern crate peel_ip;
extern crate curl;
extern crate pnet;

mod common;
use common::{CommStore,Communication};

use curl::easy::Easy;
use std::{env,process};
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;

use peel_ip::prelude::*;
use pnet::datalink::NetworkInterface;
use pnet::datalink::Channel::Ethernet;

fn main() {
	let failover = || {
		eprintln!("[!] Usage:\n\tclient <interface>\n\tclient --file <file.pcap>");
		process::exit(1);
	};

	let first_arg = env::args().nth(1).unwrap_or_else(&failover);
	let mut file_mode = false;

	let channel = if first_arg == "--file" {
		file_mode = true;
		let file_location = env::args().nth(2).unwrap_or_else(&failover);
		pnet::datalink::pcap::from_file(&Path::new(&file_location), Default::default())
	} else {
		pnet::datalink::channel(&pnet::datalink::interfaces().into_iter()
			.find(|i: &NetworkInterface| i.name == first_arg).unwrap_or_else(|| {
				eprintln!("[!] That interface does not exist");
				process::exit(1);
			}), Default::default())
	};

	let (_, mut rx) = match channel {
		Ok(Ethernet(tx, rx)) => (tx, rx),
		Ok(_) => panic!("[!] Unhandled channel type"),
		Err(e) => panic!("[!] Unable to create channel: {}", e),
	};

	let mut data = CommStore::new();
	let mut peel = PeelIp::default();
	let mut easy = Easy::new();
	let mut count = 0;

	loop {
		match rx.next() {
			Ok(packet) => {
				let result = peel.traverse(packet, vec![]).result;
				let (mut src, mut dst) = (IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)));
				let (mut src_group, mut dst_group) = ("desktop", "desktop");
				let mut packet_type = "unknown";

				for i in result {
					// Layer 1
					if i.downcast_ref::<EthernetPacket>().is_some() { packet_type = "Ethernet"; }

					// Layer 2
					else if let Some(packet) = i.downcast_ref::<ArpPacket>() {
						packet_type = "Arp";
						src = IpAddr::V4(packet.sender_protocol_address);
						dst = IpAddr::V4(packet.target_protocol_address);
					}

					// Layer 3
					else if let Some(packet) = i.downcast_ref::<Ipv4Packet>() {
						packet_type = "IPv4";
						src = IpAddr::V4(packet.src);
						dst = IpAddr::V4(packet.dst);
                        if packet.src.is_broadcast() { src_group = "broadcast"; }
		        		if packet.dst.is_broadcast() { dst_group = "broadcast"; }
					}
					else if let Some(packet) = i.downcast_ref::<Ipv6Packet>() {
						packet_type = "IPv6";
						src = IpAddr::V6(packet.src);
						dst = IpAddr::V6(packet.dst);
					}
					else if i.downcast_ref::<IcmpPacket>().is_some() { packet_type = "ICMP"; }
					else if i.downcast_ref::<Icmpv6Packet>().is_some() { packet_type = "ICMPv6"; }
					else if i.downcast_ref::<EapolPacket>().is_some() { packet_type = "EAPOL"; }

					// Layer 4
					else if i.downcast_ref::<UdpPacket>().is_some() { packet_type = "UDP"; }
					else if i.downcast_ref::<TcpPacket>().is_some() { packet_type = "TCP"; }
					else if i.downcast_ref::<IgmpPacket>().is_some() { packet_type = "IGMP"; }

					// Layer 7
					else if i.downcast_ref::<DhcpPacket>().is_some() { packet_type = "DHCP"; }
					else if i.downcast_ref::<Dhcpv6Packet>().is_some() { packet_type = "DHCPv6"; }
					else if i.downcast_ref::<DnsPacket>().is_some() { packet_type = "DNS"; }
					else if i.downcast_ref::<HttpPacket>().is_some() { packet_type = "HTTP"; }
					else if i.downcast_ref::<NtpPacket>().is_some() { packet_type = "NTP"; }
					else if i.downcast_ref::<SsdpPacket>().is_some() { packet_type = "SSDP"; }
					else if i.downcast_ref::<TlsPacket>().is_some() { packet_type = "TLS"; }
					else if i.downcast_ref::<NatpmpPacket>().is_some() { packet_type = "NAT-PMP"; }
					else if i.downcast_ref::<RadiusPacket>().is_some() { packet_type = "RADIUS"; }

					else { println!("{:?}", packet); }
				}

				if src == dst { continue; }

				if src.is_multicast() { src_group = "broadcast"; }
				if dst.is_multicast() { dst_group = "broadcast"; }

				if src.is_global() { src_group = "internet"; }
				if dst.is_global() { dst_group = "internet"; }

				if src.is_unspecified() { src_group = "broadcast"; }
				if dst.is_unspecified() { dst_group = "broadcast"; }

				if src.is_documentation() { src_group = "other"; }
				if dst.is_documentation() { dst_group = "other"; }

				data.add(Communication {
					src: src.to_string(),
					src_group: src_group.to_string(),
					dst: dst.to_string(),
					dst_group: dst_group.to_string(),
					typ: vec!(packet_type.to_string()),
					value: 1,
				});
				count += 1;

				if count > 100 || file_mode {
					let data_to_send = serde_json::to_string(&data).unwrap();
					let mut data2 = data_to_send.as_bytes();

					easy.url("http://[::]:3000/new").unwrap();
					easy.post(true).unwrap();
					easy.post_field_size(data2.len() as u64).unwrap();

					let mut transfer = easy.transfer();
					transfer.read_function(|buf| { Ok(data2.read(buf).unwrap_or(0)) }).unwrap();
					transfer.perform().unwrap();

					data.data.clear();
					count = 0;
				}
			},
			Err(_) => {
				if file_mode {
					process::exit(0);
				}
				continue;
			}
		}
	}
}
