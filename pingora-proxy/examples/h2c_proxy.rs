//! a proxy app for test
//! test h2c proxy ,include grpc

use std::path::PathBuf;
use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use clap::Parser;
use log::info;
use pingora_core::apps::HttpServerOptions;
use pingora_core::protocols::ALPN;
use pingora_core::{
    prelude::{background_service, HttpPeer, Opt},
    server::Server,
    Result,
};
use pingora_load_balancing::{health_check, prelude::RoundRobin, LoadBalancer};
use pingora_proxy::{ProxyHttp, Session};

struct LB {
    cli: Cli,
    lb: Arc<LoadBalancer<RoundRobin>>,
}
impl LB {
    pub fn new(cli: Cli, lb: Arc<LoadBalancer<RoundRobin>>) -> Self {
        return Self { cli, lb };
    }
}
#[async_trait]
impl ProxyHttp for LB {
    type CTX = ();
    fn new_ctx(&self) -> Self::CTX {}

    async fn upstream_peer(&self, _session: &mut Session, _ctx: &mut ()) -> Result<Box<HttpPeer>> {
        let upstream = self
            .lb
            .select(b"", 256) // hash doesn't matter
            .unwrap();

        info!("upstream peer is: {:?}", upstream);

        // set upstream protocl, otherwise it will use h1 and error while proxy to h2c service
        let mut peer = HttpPeer::new(upstream, false, "127.0.0.1".to_string());

        if self.cli.h2 {
            peer.options.alpn = ALPN::H2;
        }

        let peer = Box::new(peer);
        Ok(peer)
    }
}

pub fn main() {
    env_logger::init();

    let cli = Cli::parse();
    info!("cli: {:?}",&cli);
    // read command line arguments
    let mut opt = Opt::default();
    opt.conf = cli.config.clone();
    let mut my_server = Server::new(Some(opt)).unwrap();
    my_server.bootstrap();

    // only proxy to default tonic server port
    let upstream_addr = cli.upstream.as_str();
    info!("proxy to : {}", upstream_addr);
    let mut upstreams = LoadBalancer::try_from_iter([upstream_addr]).unwrap();
    let hc = health_check::TcpHealthCheck::new();
    upstreams.set_health_check(hc);
    upstreams.health_check_frequency = Some(Duration::from_secs(1));

    let background = background_service("health check", upstreams);
    let upstreams = background.task();

    let listen = cli.listen.as_str();
    let mut lb = pingora_proxy::http_proxy_service(
        &my_server.configuration,
        LB::new(cli.clone(), upstreams),
    );
    lb.add_tcp(listen);
    if let Some(http_logic) = lb.app_logic_mut() {
        let mut http_server_options = HttpServerOptions::default();
        if cli.h2 {
            http_server_options.h2c = true;
        } else {
            http_server_options.h2c = false;
        }

        http_logic.server_options = Some(http_server_options);
    }
    info!("listen: {}", listen);

    my_server.add_service(lb);
    my_server.add_service(background);
    my_server.run_forever();
}

#[derive(Parser, Clone, Debug)]
struct Cli {
    /// upstream addr, for example: 127.0.0.1:3000
    #[clap(short, long)]
    pub upstream: String,
    /// listen addr, for example: 0.0.0.0:2080
    #[clap(short, long)]
    pub listen: String,
    /// if upstream h2,default false
    #[clap(short, long, action)]
    pub h2: bool,
    /// pingora ServerConf
    #[clap(short, long)]
    pub config: Option<String>
}
