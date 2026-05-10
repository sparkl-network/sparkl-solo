use libp2p::kad;
use libp2p::mdns;
use libp2p::ping;
use libp2p::swarm::NetworkBehaviour;
use libp2p::{identify, PeerId};

#[derive(NetworkBehaviour)]
#[behaviour(to_swarm = "SparklEvent")]
pub struct SparklNetworkBehaviour {
    pub kademlia: kad::Behaviour<kad::store::MemoryStore>,
    pub mdns: mdns::tokio::Behaviour,
    pub identify: identify::Behaviour,
    pub ping: ping::Behaviour,
}

#[derive(Debug)]
pub enum SparklEvent {
    Kademlia(kad::Event),
    Mdns(mdns::Event),
    Identify(identify::Event),
    Ping(ping::Event),
}

impl From<kad::Event> for SparklEvent {
    fn from(value: kad::Event) -> Self {
        Self::Kademlia(value)
    }
}

impl From<mdns::Event> for SparklEvent {
    fn from(value: mdns::Event) -> Self {
        Self::Mdns(value)
    }
}

impl From<identify::Event> for SparklEvent {
    fn from(value: identify::Event) -> Self {
        Self::Identify(value)
    }
}

impl From<ping::Event> for SparklEvent {
    fn from(value: ping::Event) -> Self {
        Self::Ping(value)
    }
}

pub fn build_behaviour(
    local_peer_id: PeerId,
    public_key: libp2p::identity::PublicKey,
) -> anyhow::Result<SparklNetworkBehaviour> {
    let mut kademlia =
        kad::Behaviour::new(local_peer_id, kad::store::MemoryStore::new(local_peer_id));
    kademlia.set_mode(Some(kad::Mode::Server));

    let mdns = mdns::tokio::Behaviour::new(mdns::Config::default(), local_peer_id)?;
    let identify = identify::Behaviour::new(identify::Config::new(
        "sparkl/0.1.0".to_string(),
        public_key,
    ));
    let ping = ping::Behaviour::new(ping::Config::new());

    Ok(SparklNetworkBehaviour {
        kademlia,
        mdns,
        identify,
        ping,
    })
}
