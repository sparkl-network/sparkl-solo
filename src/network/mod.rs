use std::collections::HashSet;
use std::fs;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use futures::StreamExt;
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

    let handle = SwarmHandle {
        peer_id: local_peer_id.to_string(),
        listen_addrs: config.listen_addrs.clone(),
    };
    let local_peer_id_str = handle.peer_id.clone();

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

#[cfg(unix)]
fn set_private_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path) {}
