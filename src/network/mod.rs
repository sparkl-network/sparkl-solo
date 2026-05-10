use std::collections::HashSet;
use std::fs;
use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use futures::StreamExt;
use libp2p::multiaddr::Protocol;
use libp2p::swarm::SwarmEvent;
use libp2p::{mdns, Multiaddr, PeerId, SwarmBuilder};
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::config::NetworkConfig;
use crate::identity::NodeIdentity;

pub mod behaviour;
use behaviour::{build_behaviour, SparklEvent};

#[derive(Debug)]
pub enum SwarmCommand {
    GetKnownPeers(tokio::sync::oneshot::Sender<Vec<String>>),
}

#[derive(Debug, Clone)]
pub struct SwarmHandle {
    pub peer_id: String,
    pub listen_addrs: Vec<String>,
}

pub async fn start_swarm(
    _identity: &NodeIdentity,
    config: &NetworkConfig,
    data_dir: &Path,
) -> Result<(SwarmHandle, mpsc::Sender<SwarmCommand>)> {
    let (tx, mut rx) = mpsc::channel::<SwarmCommand>(8);
    let local_key = load_or_generate_swarm_key(data_dir)?;
    let local_peer_id = PeerId::from(local_key.public());
    persist_peer_id_details(data_dir, local_peer_id)?;
    let behaviour = build_behaviour(local_peer_id, local_key.public())?;

    let mut swarm = SwarmBuilder::with_existing_identity(local_key)
        .with_tokio()
        .with_tcp(
            libp2p::tcp::Config::default(),
            libp2p::noise::Config::new,
            libp2p::yamux::Config::default,
        )
        .context("failed to configure tcp transport")?
        .with_quic()
        .with_behaviour(|_| behaviour)
        .context("failed to create network behaviour")?
        .build();

    for addr in &config.listen_addrs {
        let parsed: Multiaddr = addr
            .parse()
            .map_err(|e| anyhow!("invalid listen addr `{addr}`: {e}"))?;
        if let Err(err) = swarm.listen_on(parsed.clone()) {
            warn!(%err, %parsed, "failed to listen on address");
        }
    }

    for peer in &config.bootstrap_peers {
        match peer.parse::<Multiaddr>() {
            Ok(addr) => {
                if let Err(err) = swarm.dial(addr.clone()) {
                    warn!(%err, %addr, "bootstrap dial failed");
                }
            }
            Err(err) => warn!(%err, %peer, "invalid bootstrap address"),
        }
    }

    for public_addr in &config.public_addr {
        match public_addr.parse::<Multiaddr>() {
            Ok(parsed) => {
                let target_peer = peer_id_from_addr(&parsed).unwrap_or(local_peer_id);
                let kad_addr = strip_peer_component(&parsed);
                swarm
                    .behaviour_mut()
                    .kademlia
                    .add_address(&target_peer, kad_addr.clone());
                swarm.add_external_address(parsed.clone());
                info!(
                    local_peer=%local_peer_id,
                    target_peer=%target_peer,
                    %parsed,
                    %kad_addr,
                    "registered public address for DHT and external advertisement"
                );
            }
            Err(err) => warn!(%err, %public_addr, "invalid public address"),
        }
    }

    let handle = SwarmHandle {
        peer_id: local_peer_id.to_string(),
        listen_addrs: config.listen_addrs.clone(),
    };
    let local_peer_id_str = handle.peer_id.clone();
    let allow_non_globals_in_dht = config.allow_non_globals_in_dht;

    tokio::spawn(async move {
        let mut known_peers: HashSet<PeerId> = HashSet::new();
        loop {
            tokio::select! {
                cmd = rx.recv() => {
                    match cmd {
                        Some(SwarmCommand::GetKnownPeers(reply_tx)) => {
                            let peers = known_peers.iter().map(ToString::to_string).collect::<Vec<_>>();
                            let _ = reply_tx.send(peers);
                        }
                        None => break,
                    }
                }
                event = swarm.select_next_some() => {
                    match event {
                        SwarmEvent::Behaviour(SparklEvent::Mdns(mdns::Event::Discovered(peers))) => {
                            for (peer, addr) in peers {
                                info!(
                                    local_peer=%local_peer_id_str,
                                    discovered_peer=%peer,
                                    %addr,
                                    "mDNS discovered peer candidate"
                                );
                            }
                        }
                        SwarmEvent::Behaviour(SparklEvent::Mdns(mdns::Event::Expired(peers))) => {
                            for (peer, addr) in peers {
                                info!(
                                    local_peer=%local_peer_id_str,
                                    expired_peer=%peer,
                                    %addr,
                                    "mDNS peer expired"
                                );
                                swarm.behaviour_mut().kademlia.remove_address(&peer, &addr);
                            }
                        }
                        SwarmEvent::Behaviour(SparklEvent::Identify(event)) => {
                            if let libp2p::identify::Event::Received { peer_id: remote_peer_id, info, .. } = event {
                                let protocol_version = info.protocol_version.clone();
                                let agent_version = info.agent_version.clone();
                                let is_sparkl_peer = protocol_version.starts_with("sparkl/");
                                if is_sparkl_peer {
                                    known_peers.insert(remote_peer_id);
                                    for addr in info.listen_addrs {
                                        if !is_dht_address_allowed(&addr, allow_non_globals_in_dht) {
                                            info!(
                                                local_peer=%local_peer_id_str,
                                                identified_peer=%remote_peer_id,
                                                %addr,
                                                "ignored non-global address for DHT"
                                            );
                                            continue;
                                        }
                                        info!(
                                            local_peer=%local_peer_id_str,
                                            identified_peer=%remote_peer_id,
                                            %addr,
                                            protocol_version=%protocol_version,
                                            agent_version=%agent_version,
                                            "identify accepted sparkl peer"
                                        );
                                        swarm.behaviour_mut().kademlia.add_address(&remote_peer_id, addr);
                                    }
                                } else {
                                    info!(
                                        local_peer=%local_peer_id_str,
                                        identified_peer=%remote_peer_id,
                                        protocol_version=%protocol_version,
                                        agent_version=%agent_version,
                                        "identify ignored non-sparkl peer"
                                    );
                                    known_peers.remove(&remote_peer_id);
                                }
                            }
                        }
                        SwarmEvent::Behaviour(SparklEvent::Ping(ping_event)) => {
                            let peer = ping_event.peer;
                            if ping_event.result.is_err() {
                                info!(local_peer=%local_peer_id_str, ping_peer=%peer, "ping failed");
                            } else {
                                info!(local_peer=%local_peer_id_str, ping_peer=%peer, "ping success");
                            }
                        }
                        SwarmEvent::Behaviour(SparklEvent::Kademlia(kad_event)) => {
                            info!(local_peer=%local_peer_id_str, ?kad_event, "kademlia DHT event");
                        }
                        SwarmEvent::NewListenAddr { address, .. } => {
                            info!(local_peer=%local_peer_id_str, %address, "swarm listening");
                        }
                        SwarmEvent::ConnectionEstablished { peer_id: remote_peer_id, .. } => {
                            info!(
                                local_peer=%local_peer_id_str,
                                connected_peer=%remote_peer_id,
                                "connection established"
                            );
                        }
                        SwarmEvent::ConnectionClosed { peer_id: remote_peer_id, .. } => {
                            info!(
                                local_peer=%local_peer_id_str,
                                disconnected_peer=%remote_peer_id,
                                "connection closed (peer retained in known set)"
                            );
                        }
                        SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                            info!(
                                local_peer=%local_peer_id_str,
                                dial_peer=?peer_id,
                                %error,
                                "outgoing connection error"
                            );
                        }
                        SwarmEvent::IncomingConnectionError { local_addr, send_back_addr, error, .. } => {
                            info!(
                                local_peer=%local_peer_id_str,
                                %local_addr,
                                %send_back_addr,
                                %error,
                                "incoming connection error"
                            );
                        }
                        _ => {}
                    }
                }
            }
        }
    });

    Ok((handle, tx))
}

fn load_or_generate_swarm_key(data_dir: &Path) -> Result<libp2p::identity::Keypair> {
    let network_dir = data_dir.join("network");
    fs::create_dir_all(&network_dir).context("failed to create network data dir")?;
    let key_path = network_dir.join("secret_ed25519");

    if key_path.exists() {
        let bytes = fs::read(&key_path).context("failed to read persisted swarm key")?;
        let keypair = libp2p::identity::Keypair::from_protobuf_encoding(&bytes)
            .context("failed to decode persisted swarm key")?;
        return Ok(keypair);
    }

    let keypair = libp2p::identity::Keypair::generate_ed25519();
    let encoded = keypair
        .to_protobuf_encoding()
        .context("failed to encode swarm key")?;
    fs::write(&key_path, encoded).context("failed to persist generated swarm key")?;
    set_private_permissions(&key_path);
    Ok(keypair)
}

fn persist_peer_id_details(data_dir: &Path, peer_id: PeerId) -> Result<()> {
    let network_dir = data_dir.join("network");
    fs::create_dir_all(&network_dir).context("failed to create network data dir")?;
    let peer_id_path = network_dir.join("peer_id");
    fs::write(&peer_id_path, format!("{peer_id}\n")).context("failed to persist peer id")?;
    Ok(())
}

fn peer_id_from_addr(addr: &Multiaddr) -> Option<PeerId> {
    addr.iter().find_map(|p| {
        if let Protocol::P2p(peer_id) = p {
            Some(peer_id)
        } else {
            None
        }
    })
}

fn strip_peer_component(addr: &Multiaddr) -> Multiaddr {
    let mut out = Multiaddr::empty();
    for proto in addr.iter() {
        if !matches!(proto, Protocol::P2p(_)) {
            out.push(proto);
        }
    }
    out
}

fn is_dht_address_allowed(addr: &Multiaddr, allow_non_globals_in_dht: bool) -> bool {
    if allow_non_globals_in_dht {
        return true;
    }

    for proto in addr.iter() {
        match proto {
            Protocol::Ip4(ip) => return is_global_ipv4(ip),
            Protocol::Ip6(ip) => return is_global_ipv6(ip),
            Protocol::Dns(_) | Protocol::Dns4(_) | Protocol::Dns6(_) | Protocol::Dnsaddr(_) => {
                return true
            }
            _ => continue,
        }
    }
    true
}

fn is_global_ipv4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    let is_shared_cgnat = octets[0] == 100 && (octets[1] & 0b1100_0000) == 0b0100_0000;
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || is_shared_cgnat
        || octets[0] == 0)
}

fn is_global_ipv6(ip: std::net::Ipv6Addr) -> bool {
    let ip = IpAddr::V6(ip);
    !(ip.is_unspecified()
        || ip.is_loopback()
        || matches!(ip, IpAddr::V6(v6) if v6.is_unique_local() || v6.is_unicast_link_local() || v6.is_multicast()))
}

#[cfg(unix)]
fn set_private_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path) {}
