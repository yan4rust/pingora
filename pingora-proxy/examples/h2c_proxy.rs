//! test h2c proxy ,include grpc

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use clap::Parser;
use log::info;
use pingora_core::apps::HttpServerOptions;
use pingora_core::protocols::ALPN;
use pingora_core::{prelude::{background_service, HttpPeer, Opt}, server::Server, Result};
use pingora_load_balancing::{health_check, prelude::RoundRobin, LoadBalancer};
use pingora_proxy::{ProxyHttp, Session};

pub struct LB(Arc<LoadBalancer<RoundRobin>>);
#[async_trait]
impl ProxyHttp for LB {
    type CTX = ();
    fn new_ctx(&self) -> Self::CTX {}

    async fn upstream_peer(&self, _session: &mut Session, _ctx: &mut ()) -> Result<Box<HttpPeer>> {
        let upstream = self
            .0
            .select(b"", 256) // hash doesn't matter
            .unwrap();

        info!("upstream peer is: {:?}", upstream);

        // set upstream protocl, otherwise it will use h1 and error while proxy to h2c service
        let mut peer = HttpPeer::new(upstream, false, "127.0.0.1".to_string());
        peer.options.alpn = ALPN::H2;
        let peer = Box::new(peer);
        Ok(peer)
    }

}

pub fn main() {
    env_logger::init();

    // read command line arguments
    let opt = Opt::parse();
    let mut my_server = Server::new(Some(opt)).unwrap();
    my_server.bootstrap();

    // only proxy to default tonic server port
    let upstream_addr = "127.0.0.1:50051";
    info!("proxy to : {}", upstream_addr);
    let mut upstreams = LoadBalancer::try_from_iter(["127.0.0.1:50051"]).unwrap();
    let hc = health_check::TcpHealthCheck::new();
    upstreams.set_health_check(hc);
    upstreams.health_check_frequency = Some(Duration::from_secs(1));

    let background = background_service("health check", upstreams);
    let upstreams = background.task();

    let listen = "0.0.0.0:6189";
    let mut lb = pingora_proxy::http_proxy_service(&my_server.configuration, LB(upstreams));
    lb.add_tcp(listen);
    if let Some(http_logic) = lb.app_logic_mut() {
        let mut http_server_options = HttpServerOptions::default();
        http_server_options.h2c = true;
        http_logic.server_options = Some(http_server_options);
    }
    info!("listen: {}", listen);

    my_server.add_service(lb);
    my_server.add_service(background);
    my_server.run_forever();
}
