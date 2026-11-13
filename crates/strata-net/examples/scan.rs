//! cargo run --example scan -- 192.168.1.0/24
use strata_net::{cidr, scan};

#[tokio::main]
async fn main() {
    let range = cidr::parse(&std::env::args().nth(1).unwrap_or("127.0.0.1".into())).expect("range");
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(scan::run(range, scan::Options::default(), tx, scan::Stop::default()));
    while let Some(e) = rx.recv().await {
        match e {
            scan::Event::Progress { .. } => {}
            e => println!("{}", serde_json::to_string(&e).unwrap()),
        }
    }
}
