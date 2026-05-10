use std::collections::HashSet;

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
) -> Result<(SwarmHandle, mpsc::Sender<SwarmCommand>)> {
    let (tx, mut rx) = mpsc::channel::<SwarmCommand>(8);
    let local_key = libp2p::identity::Keypair::generate_ed25519();
    let local_peer_id = PeerId::from(local_key.public());
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
    let peer_id = handle.peer_id.clone();

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
                                known_peers.insert(peer);
                                swarm.behaviour_mut().kademlia.add_address(&peer, addr);
                            }
                        }
                        SwarmEvent::Behaviour(SparklEvent::Mdns(mdns::Event::Expired(peers))) => {
                            for (peer, addr) in peers {
                                swarm.behaviour_mut().kademlia.remove_address(&peer, &addr);
                            }
                        }
                        SwarmEvent::Behaviour(SparklEvent::Identify(event)) => {
                            if let libp2p::identify::Event::Received { peer_id, info, .. } = event {
                                known_peers.insert(peer_id);
                                for addr in info.listen_addrs {
                                    swarm.behaviour_mut().kademlia.add_address(&peer_id, addr);
                                }
                            }
                        }
                        SwarmEvent::Behaviour(SparklEvent::Ping(ping_event)) => {
                            let peer = ping_event.peer;
                            if ping_event.result.is_err() {
                                known_peers.remove(&peer);
                            } else {
                                known_peers.insert(peer);
                            }
                        }
                        SwarmEvent::Behaviour(SparklEvent::Kademlia(kad_event)) => {
                            info!(?kad_event, %peer_id, "kademlia event");
                        }
                        SwarmEvent::NewListenAddr { address, .. } => {
                            info!(%peer_id, %address, "swarm listening");
                        }
                        SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                            known_peers.insert(peer_id);
                        }
                        SwarmEvent::ConnectionClosed { peer_id, .. } => {
                            known_peers.remove(&peer_id);
                        }
                        _ => {}
                    }
                }
            }
        }
    });

    Ok((handle, tx))
}
